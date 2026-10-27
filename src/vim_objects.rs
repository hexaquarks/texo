use std::{ops::Range, sync::OnceLock};

use ratatui_textarea::{DataCursor, TextArea};
use regex::Regex;

#[derive(Clone, Copy)]
pub(super) enum ObjectScope {
    Inner,
    Around,
}

impl ObjectScope {
    pub(super) fn from_key(character: char) -> Option<Self> {
        match character {
            'i' => Some(Self::Inner),
            'a' => Some(Self::Around),
            _ => None,
        }
    }
}

pub(super) struct ObjectRange {
    pub(super) start: DataCursor,
    pub(super) end: DataCursor,
}

pub(super) fn text_object(
    text: &TextArea<'_>,
    scope: ObjectScope,
    key: char,
) -> Option<ObjectRange> {
    let source = text.lines().join("\n");
    let cursor = text.cursor();
    let line = &text.lines()[cursor.0];
    let column = cursor.1.min(line.chars().count().saturating_sub(1));
    let line_start: usize = text.lines()[..cursor.0]
        .iter()
        .map(|line| line.len() + 1)
        .sum();
    let offset = line_start
        + line
            .char_indices()
            .nth(column)
            .map_or(0, |(offset, _)| offset);

    let range = match key {
        'w' | 'W' => word_range(&source, offset, scope, key == 'W'),
        '(' | ')' | 'b' => block_range(&source, offset, scope, '(', ')'),
        '{' | '}' | 'B' => block_range(&source, offset, scope, '{', '}'),
        '[' | ']' => block_range(&source, offset, scope, '[', ']'),
        '<' | '>' => block_range(&source, offset, scope, '<', '>'),
        '\'' | '"' | '`' => quote_range(&source, offset, scope, key),
        _ => None,
    }?;

    Some(ObjectRange {
        start: source_position(&source, range.start),
        end: source_position(&source, range.end),
    })
}

fn source_position(source: &str, offset: usize) -> DataCursor {
    let prefix = &source[..offset];
    let row = prefix
        .chars()
        .filter(|character| *character == '\n')
        .count();
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count();
    DataCursor(row, column)
}

fn word_pattern(big_word: bool) -> &'static Regex {
    // Vim words separate keyword characters from punctuation; combining marks stay with letters.
    const WORD_PATTERN: &str =
        r"[\p{Alphabetic}\p{Mark}\p{Number}_]+|[^\p{Alphabetic}\p{Mark}\p{Number}_\s]+|[^\S\n]+";
    static WORD: OnceLock<Regex> = OnceLock::new();
    static BIG_WORD: OnceLock<Regex> = OnceLock::new();

    if big_word {
        BIG_WORD.get_or_init(|| Regex::new(r"\S+|[^\S\n]+").expect("Valid WORD pattern"))
    } else {
        WORD.get_or_init(|| Regex::new(WORD_PATTERN).expect("Valid word pattern"))
    }
}

fn word_range(
    source: &str,
    cursor: usize,
    scope: ObjectScope,
    big_word: bool,
) -> Option<Range<usize>> {
    let pattern = word_pattern(big_word);
    let word = pattern
        .find_iter(source)
        .find(|word| word.range().contains(&cursor))?;
    let range = word.range();
    if matches!(scope, ObjectScope::Inner) {
        return Some(range);
    }

    if word.as_str().chars().all(char::is_whitespace) {
        let next_word = pattern
            .find_iter(&source[range.end..])
            .find(|word| !word.as_str().chars().all(char::is_whitespace))?;
        return Some(range.start..range.end + next_word.end());
    }

    Some(include_surrounding_space(source, range))
}

fn include_surrounding_space(source: &str, range: Range<usize>) -> Range<usize> {
    let inline_space = |character: char| character != '\n' && character.is_whitespace();
    let after = &source[range.end..];
    let end = source.len() - after.trim_start_matches(inline_space).len();
    if end > range.end {
        return range.start..end;
    }

    let start = source[..range.start].trim_end_matches(inline_space).len();
    start..range.end
}

fn block_range(
    source: &str,
    cursor: usize,
    scope: ObjectScope,
    opening: char,
    closing: char,
) -> Option<Range<usize>> {
    let mut openings = Vec::new();
    let mut pairs = Vec::new();
    let mut escaped = false;
    for (offset, character) in source.char_indices() {
        if character == '\\' {
            escaped = !escaped;
            continue;
        }
        if !escaped {
            if character == opening {
                openings.push(offset);
            } else if character == closing
                && let Some(start) = openings.pop()
            {
                pairs.push(start..offset + character.len_utf8());
            }
        }
        escaped = false;
    }

    let range = containing_or_next_pair(&pairs, cursor)?;
    if matches!(scope, ObjectScope::Around) {
        return Some(range);
    }

    // Inner blocks omit an opening newline and indentation on a delimiter-only closing line.
    let mut start = range.start + opening.len_utf8();
    let mut end = range.end - closing.len_utf8();
    if source[start..end].starts_with('\n') {
        start += '\n'.len_utf8();
    }
    let closing_line = source[..end]
        .rfind('\n')
        .map(|offset| offset + '\n'.len_utf8());
    if let Some(line_start) = closing_line
        && line_start >= start
        && source[line_start..end].chars().all(char::is_whitespace)
    {
        end = line_start;
    }
    Some(start..end)
}

fn containing_or_next_pair(pairs: &[Range<usize>], cursor: usize) -> Option<Range<usize>> {
    pairs
        .iter()
        .filter(|range| range.contains(&cursor))
        .min_by_key(|range| range.len())
        .or_else(|| {
            pairs
                .iter()
                .filter(|range| range.start >= cursor)
                .min_by_key(|range| range.start)
        })
        .cloned()
}

fn quote_range(
    source: &str,
    cursor: usize,
    scope: ObjectScope,
    quote: char,
) -> Option<Range<usize>> {
    let line_start = source[..cursor]
        .rfind('\n')
        .map_or(0, |offset| offset + '\n'.len_utf8());
    let line_end = source[cursor..]
        .find('\n')
        .map_or(source.len(), |offset| cursor + offset);
    let mut opening = None;
    let mut pairs = Vec::new();
    let mut escaped = false;
    for (column, character) in source[line_start..line_end].char_indices() {
        if character == '\\' {
            escaped = !escaped;
            continue;
        }
        if character == quote && !escaped {
            let offset = line_start + column;
            if let Some(start) = opening.take() {
                pairs.push(start..offset + character.len_utf8());
            } else {
                opening = Some(offset);
            }
        }
        escaped = false;
    }

    let range = containing_or_next_pair(&pairs, cursor)?;
    Some(match scope {
        ObjectScope::Inner => range.start + quote.len_utf8()..range.end - quote.len_utf8(),
        ObjectScope::Around => include_surrounding_space(source, range),
    })
}
