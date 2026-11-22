use std::ops::Range;

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use reqwest::Url;
use texo::theme;
use unicode_segmentation::UnicodeSegmentation;

const TAB_SPACES: &str = "    ";
const LIST_INDENT: &str = "  ";
const BLOCK_BREAKS: usize = 2;

pub(super) struct Markdown {
    pub(super) text: String,
    pub(super) styles: Vec<StyledText>,
    pub(super) code_blocks: Vec<Range<usize>>,
    links: Vec<Link>,
}

pub(super) struct StyledText {
    pub(super) bytes: Range<usize>,
    pub(super) style: Style,
}

struct Link {
    bytes: Range<usize>,
    url: Url,
}

impl Markdown {
    pub(super) fn parse(source: &str) -> Self {
        let mut renderer = Renderer::default();
        let options =
            Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;

        for event in Parser::new_ext(source, options) {
            renderer.render(event);
        }

        renderer.finish()
    }

    pub(super) fn link_at(&self, byte: usize) -> Option<&Url> {
        let index = self.links.partition_point(|link| link.bytes.end <= byte);
        let link = self.links.get(index)?;

        link.bytes.contains(&byte).then_some(&link.url)
    }
}

#[derive(Default)]
struct Renderer<'a> {
    text: String,
    styles: Vec<StyledText>,
    style_stack: Vec<Style>,
    code_blocks: Vec<Range<usize>>,
    code_start: usize,
    links: Vec<Link>,
    destinations: Vec<Destination<'a>>,
    lists: Vec<Option<u64>>,
    quote_depth: usize,
    pending_breaks: usize,
    table_cells: usize,
}

struct Destination<'a> {
    text: CowStr<'a>,
    url: Option<Url>,
    label_start: usize,
}

impl<'a> Renderer<'a> {
    fn render(&mut self, event: Event<'a>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.write(&text),
            Event::Code(code) => {
                self.push_style(code_style());
                self.write(&code);
                self.style_stack.pop();
            }
            Event::SoftBreak | Event::HardBreak => self.write("\n"),
            Event::Rule => {
                self.space_after_block();
                self.write("───");
                self.space_after_block();
            }
            Event::TaskListMarker(checked) => {
                self.write(if checked { "[x] " } else { "[ ] " });
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'a>) {
        match tag {
            Tag::Heading { .. } => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            Tag::BlockQuote(_) => {
                self.space_after_block();
                self.quote_depth += 1;
            }
            Tag::CodeBlock(_) => {
                self.space_after_block();
                self.code_start = self.text.len();
                self.push_style(code_style());
            }
            Tag::List(first) => {
                if !self.lists.is_empty() && !self.text.is_empty() {
                    self.pending_breaks = 1;
                }
                self.lists.push(first);
            }
            Tag::Item => self.start_item(),
            Tag::Emphasis => self.push_style(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } => self.start_destination(dest_url, true),
            Tag::Image { dest_url, .. } => self.start_destination(dest_url, false),
            Tag::TableHead => {
                self.table_cells = 0;
                self.push_style(Style::new().add_modifier(Modifier::BOLD));
            }
            Tag::TableRow => self.table_cells = 0,
            Tag::TableCell => {
                if self.table_cells > 0 {
                    self.write(" | ");
                }
                self.table_cells += 1;
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.style_stack.pop();
                self.space_after_block();
            }
            TagEnd::Paragraph | TagEnd::HtmlBlock | TagEnd::Table => self.space_after_block(),
            TagEnd::BlockQuote(_) => {
                self.quote_depth -= 1;
                self.space_after_block();
            }
            TagEnd::CodeBlock => {
                self.style_stack.pop();
                self.code_blocks.push(self.code_start..self.text.len());
                self.space_after_block();
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.space_after_block();
                } else {
                    self.pending_breaks = 1;
                }
            }
            TagEnd::Item | TagEnd::TableRow => self.pending_breaks = 1,
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.style_stack.pop();
            }
            TagEnd::Link | TagEnd::Image => self.end_destination(),
            TagEnd::TableHead => {
                self.style_stack.pop();
                self.pending_breaks = 1;
            }
            _ => {}
        }
    }

    fn start_item(&mut self) {
        self.flush_breaks();
        let indent = LIST_INDENT.repeat(self.lists.len().saturating_sub(1));
        self.write_quote_prefix();
        self.append(&indent);

        let marker = match self.lists.last_mut() {
            Some(Some(number)) => {
                let marker = format!("{number}. ");
                *number = number.saturating_add(1);
                marker
            }
            _ => "• ".into(),
        };
        self.append(&marker);
    }

    fn start_destination(&mut self, text: CowStr<'a>, clickable: bool) {
        self.flush_breaks();
        self.write_prefix();

        let url = if clickable { web_url(&text) } else { None };
        let style = if url.is_some() {
            Style::new()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::UNDERLINED)
        } else {
            Style::new()
        };

        self.push_style(style);
        self.destinations.push(Destination {
            text,
            url,
            label_start: self.text.len(),
        });
    }

    fn end_destination(&mut self) {
        let Some(destination) = self.destinations.pop() else {
            return;
        };
        let label = &self.text[destination.label_start..];

        if label.is_empty() {
            self.write(&destination.text);
        } else if !destination.text.is_empty() && label != destination.text.as_ref() {
            self.write(" (");
            self.write(&destination.text);
            self.write(")");
        }

        self.style_stack.pop();
        if let Some(url) = destination.url {
            self.links.push(Link {
                bytes: destination.label_start..self.text.len(),
                url,
            });
        }
    }

    fn push_style(&mut self, style: Style) {
        let parent = self.style_stack.last().copied().unwrap_or_default();
        self.style_stack.push(parent.patch(style));
    }

    fn space_after_block(&mut self) {
        self.pending_breaks = BLOCK_BREAKS;
    }

    fn flush_breaks(&mut self) {
        let pending = std::mem::take(&mut self.pending_breaks);
        if pending == 0 || self.text.is_empty() {
            return;
        }

        let existing = self
            .text
            .bytes()
            .rev()
            .take(pending)
            .take_while(|&byte| byte == b'\n')
            .count();
        self.append(&"\n".repeat(pending - existing));
    }

    fn write_prefix(&mut self) {
        if self.text.is_empty() || self.text.ends_with('\n') {
            self.write_quote_prefix();
            let indent = LIST_INDENT.repeat(self.lists.len());
            self.append(&indent);
        }
    }

    fn write_quote_prefix(&mut self) {
        let prefix = "│ ".repeat(self.quote_depth);
        self.append(&prefix);
    }

    fn write(&mut self, text: &str) {
        self.flush_breaks();

        for line in text.split_inclusive('\n') {
            if line != "\n" {
                self.write_prefix();
            }

            // Terminal control characters in provider text and HTML stay inert.
            let mut start = 0;
            for (index, character) in line.char_indices() {
                if character == '\n' || !character.is_control() {
                    continue;
                }

                self.append(&line[start..index]);
                self.append(if character == '\t' { TAB_SPACES } else { "�" });
                start = index + character.len_utf8();
            }
            self.append(&line[start..]);
        }
    }

    fn append(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }

        let start = self.text.len();
        self.text.push_str(text);
        let end = self.text.len();
        let style = self.style_stack.last().copied().unwrap_or_default();

        if let Some(previous) = self.styles.last_mut()
            && previous.style == style
        {
            previous.bytes.end = end;
        } else {
            self.styles.push(StyledText {
                bytes: start..end,
                style,
            });
        }
    }

    fn finish(mut self) -> Markdown {
        // A style boundary can fall between a base character and a combining mark.
        // Keep each displayed grapheme together and give it the style of its first character.
        let mut boundaries = self
            .text
            .grapheme_indices(true)
            .map(|(byte, _)| byte)
            .peekable();
        let mut start = 0;

        for span in &mut self.styles {
            while boundaries.next_if(|byte| *byte < span.bytes.end).is_some() {}

            let end = boundaries.peek().copied().unwrap_or(self.text.len());
            span.bytes = start..end;
            start = end;
        }
        self.styles.retain(|span| !span.bytes.is_empty());

        Markdown {
            text: self.text,
            styles: self.styles,
            code_blocks: self.code_blocks,
            links: self.links,
        }
    }
}

fn code_style() -> Style {
    Style::new().fg(theme::TEAL).bg(theme::ELEVATED)
}

fn web_url(destination: &str) -> Option<Url> {
    let (scheme, authority) = destination.split_once("://")?;
    let web_scheme = scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https");
    if !web_scheme
        || authority.starts_with('/')
        || destination.contains(|character: char| {
            character.is_control() || character.is_whitespace() || character == '\\'
        })
    {
        return None;
    }

    let url = Url::parse(destination).ok()?;
    // Credentials can make a URL appear to point at a different host.
    if url.host().is_none() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }

    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_common_blocks_and_keeps_code_and_html_literal() {
        let source = concat!(
            "## License check\n\n",
            "7. **MIT** and *permissive*\n",
            "   - [x] Read `LICENSE`\n",
            "8. ~~Deprecated~~\n\n",
            "> First line\n> Second line\n\n",
            "```text\n  **literal**\n\n\tfile.tex\n```\n\n",
            "| Name | Result |\n| --- | --- |\n| MIT | Pass |\n\n",
            "---\n\n",
            "<script>\u{1b}[31m</script>\n",
        );
        let markdown = Markdown::parse(source);
        assert_eq!(
            markdown.text,
            concat!(
                "License check\n\n",
                "7. MIT and permissive\n",
                "  • [x] Read LICENSE\n",
                "8. Deprecated\n\n",
                "│ First line\n│ Second line\n\n",
                "  **literal**\n\n    file.tex\n\n",
                "Name | Result\nMIT | Pass\n\n",
                "───\n\n",
                "<script>�[31m</script>\n",
            )
        );
        assert_eq!(markdown.code_blocks.len(), 1);
        assert!(markdown.styles.iter().any(|span| {
            markdown.text[span.bytes.clone()].contains("**literal**") && span.style == code_style()
        }));
    }

    #[test]
    fn keeps_reference_and_autolink_destinations_visible() {
        let markdown = Markdown::parse(concat!(
            "[License][source] and <https://example.com>\n\n",
            "[source]: https://example.com/license\n",
        ));

        assert_eq!(
            markdown.text,
            "License (https://example.com/license) and https://example.com"
        );
        assert_eq!(
            markdown.link_at(0).unwrap().as_str(),
            "https://example.com/license"
        );
        let autolink = markdown.text.rfind("https://").unwrap();
        assert_eq!(
            markdown.link_at(autolink).unwrap().host_str(),
            Some("example.com")
        );
    }

    #[test]
    fn accepts_only_unambiguous_web_urls_for_opening() {
        for url in [
            "file:///tmp/example",
            "mailto:test@example.com",
            "javascript:alert(1)",
            "//example.com",
            "https://",
            "https:///example.com",
            "https://example.com@elsewhere.example",
            "https://example.com/\nother",
            "https://example.com\\other",
        ] {
            assert!(web_url(url).is_none(), "{url}");
        }

        for url in ["http://example.com/", "https://example.com/license?x=2#mit"] {
            assert_eq!(web_url(url).unwrap().as_str(), url);
        }
    }
}
