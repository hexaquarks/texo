use std::collections::BTreeSet;
use std::fmt::Write;

use reqwest::Url;
use serde::Deserialize;
use serde_json::{Value, value::RawValue};

const MAX_CITATIONS: usize = 64;
const MAX_CITATION_URL_BYTES: usize = 8 * 1024;

pub(super) fn openai_citations(reply: &mut String, blocks: &[&Value]) {
    if !blocks.iter().any(|block| {
        block["annotations"]
            .as_array()
            .is_some_and(|annotations| !annotations.is_empty())
    }) {
        return;
    }

    let raw_response: String = blocks
        .iter()
        .filter_map(|block| block["text"].as_str())
        .collect();
    let reply_field = serde_json::from_str::<ReplyField>(&raw_response).ok();
    let mut sources = Sources::default();
    let mut block_start = 0;
    let mut remaining_citations = MAX_CITATIONS;

    for block in blocks {
        let text = block["text"].as_str().unwrap_or_default();
        let character_offsets: Vec<_> = text
            .char_indices()
            .map(|(offset, _)| offset)
            .chain([text.len()])
            .collect();

        let citations = block["annotations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|citation| citation["type"] == "url_citation")
            .take(remaining_citations);

        for citation in citations {
            let offset = reply_field.as_ref().and_then(|field| {
                field.citation_end(
                    citation,
                    &character_offsets,
                    &raw_response,
                    block_start,
                    reply,
                )
            });
            sources.add(&citation["url"], offset);
            remaining_citations -= 1;
        }

        if remaining_citations == 0 {
            break;
        }

        block_start += text.len();
    }

    sources.render(reply);
}

#[derive(Deserialize)]
struct ReplyField<'a> {
    #[serde(borrow)]
    reply: &'a RawValue,
}

impl ReplyField<'_> {
    fn citation_end(
        &self,
        citation: &Value,
        character_offsets: &[usize],
        raw_response: &str,
        block_start: usize,
        decoded_reply: &str,
    ) -> Option<usize> {
        let start = usize::try_from(citation["start_index"].as_u64()?).ok()?;
        let end = usize::try_from(citation["end_index"].as_u64()?).ok()?;
        if start > end {
            return None;
        }

        let first_byte = block_start + character_offsets.get(start)?;
        let after_last_byte = block_start + character_offsets.get(end.checked_add(1)?)?;
        let field = self.reply.get();
        // RawValue borrows the original JSON string, including the two surrounding quotes.
        let content_start = field.as_ptr().addr() - raw_response.as_ptr().addr() + 1;
        let content_end = content_start + field.len().checked_sub(2)?;

        if first_byte < content_start || after_last_byte > content_end {
            return None;
        }

        // Decoding the prefix accounts for JSON escapes and rejects a boundary inside an escape.
        let prefix = raw_response.get(content_start..after_last_byte)?;
        let decoded_prefix = serde_json::from_str::<String>(&format!("\"{prefix}\"")).ok()?;
        decoded_reply
            .starts_with(&decoded_prefix)
            .then_some(decoded_prefix.len())
    }
}

pub(super) fn anthropic_citations(
    reply: &mut String,
    blocks: &[&Value],
    earlier_messages: &[Value],
) {
    let earlier_blocks = earlier_messages
        .iter()
        .filter(|message| message["role"] == "assistant")
        .flat_map(|message| message["content"].as_array().into_iter().flatten());
    // Document indexes count successful fetched documents in conversation order.
    let fetched: Vec<_> = earlier_blocks
        .chain(blocks.iter().copied())
        .filter(|block| {
            block["type"] == "web_fetch_tool_result"
                && block["content"]["type"] == "web_fetch_result"
        })
        .map(|block| &block["content"])
        .collect();

    let mut sources = Sources::default();
    let mut offset = 0;

    for block in blocks {
        if block["type"] != "text" {
            continue;
        }

        let text = block["text"].as_str().unwrap_or_default();
        let end_of_cited_text = offset + text.trim_end().len();

        for citation in block["citations"].as_array().into_iter().flatten() {
            let url = match citation["type"].as_str() {
                Some("web_search_result_location") => Some(&citation["url"]),
                Some("char_location" | "page_location" | "content_block_location") => {
                    fetched_document_url(&fetched, citation)
                }
                _ => None,
            };

            if let Some(url) = url {
                let position = (!text.trim().is_empty()).then_some(end_of_cited_text);
                sources.add(url, position);
            }
        }

        offset += text.len();
    }

    for document in fetched {
        sources.add(&document["url"], None);
    }

    sources.render(reply);
}

fn fetched_document_url<'a>(documents: &[&'a Value], citation: &Value) -> Option<&'a Value> {
    let index = usize::try_from(citation["document_index"].as_u64()?).ok()?;
    let document = documents.get(index)?;

    // A title mismatch can indicate a document index we cannot safely associate with a URL.
    if let Some(title) = citation["document_title"].as_str()
        && document["content"]["title"] != title
    {
        return None;
    }

    Some(&document["url"])
}

#[derive(Default)]
struct Sources {
    urls: Vec<Url>,
    placements: BTreeSet<(usize, usize)>,
}

impl Sources {
    fn add(&mut self, value: &Value, offset: Option<usize>) {
        let Some(url) = value.as_str().and_then(public_url) else {
            return;
        };
        let index = match self.urls.iter().position(|known| *known == url) {
            Some(index) => index,
            None if self.urls.len() < MAX_CITATIONS => {
                self.urls.push(url);
                self.urls.len() - 1
            }
            None => return,
        };

        if let Some(offset) = offset
            && self.placements.len() < MAX_CITATIONS
        {
            self.placements.insert((offset, index));
        }
    }

    fn render(self, reply: &mut String) {
        if self.urls.is_empty() {
            return;
        }

        let mut rendered = String::new();
        let mut cursor = 0;
        let mut linked_inline = BTreeSet::new();

        for (offset, index) in self.placements {
            if let Some(before_link) = reply.get(cursor..offset) {
                rendered.push_str(before_link);
                let _ = write!(rendered, " [{}](<{}>)", index + 1, self.urls[index]);
                linked_inline.insert(index);
                cursor = offset;
            }
        }

        rendered.push_str(&reply[cursor..]);

        let mut remaining = self
            .urls
            .iter()
            .enumerate()
            .filter(|(index, _)| !linked_inline.contains(index))
            .peekable();
        if remaining.peek().is_some() {
            rendered.push_str("\n\nSources");
            for (index, url) in remaining {
                let _ = write!(
                    rendered,
                    "\n- [{}](<{}>) {}",
                    index + 1,
                    url,
                    url.domain().unwrap_or_default()
                );
            }
        }

        *reply = rendered;
    }
}

fn public_url(link: &str) -> Option<Url> {
    if link.len() > MAX_CITATION_URL_BYTES
        || link.contains(char::is_whitespace)
        || link.contains(char::is_control)
    {
        return None;
    }

    let url = Url::parse(link).ok()?;
    let domain = url.domain()?.trim_end_matches('.');
    let is_public_domain =
        domain.contains('.') && !domain.ends_with(".localhost") && !domain.ends_with(".local");

    if matches!(url.scheme(), "http" | "https")
        && is_public_domain
        && url.username().is_empty()
        && url.password().is_none()
    {
        return Some(url);
    }

    None
}
