//! Pure, bounded chat Markdown and styling for the original composer source.
//!
//! Offsets are UTF-8 byte offsets. Display spans address `Document::text`;
//! source spans address the caller's untouched input, never reconstructed text.
//! Parsing, URL recognition and all temporary storage are bounded by the 4 KiB
//! input budget. This module performs no I/O and never interprets HTML or images.

use std::ops::Range;

use crate::bounded::{Page, Text};

mod links;
mod render;
mod source;

pub use links::web_destination;

pub const MAX_SOURCE_BYTES: usize = 4096;
const MAX_TEXT_BYTES: usize = 8192;
const MAX_LINES: usize = 256;
const MAX_SPANS: usize = 512;
const MAX_LINKS: usize = 32;
const MAX_EVENTS: usize = 2048;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub text: Text<8192>,
    /// Ranges exclude line endings. An empty page with nonempty text means a
    /// literal fallback with more than 256 lines: render `text` as one block.
    pub lines: Page<Line, 256>,
    /// Sorted, disjoint runs covering the display text, including plain runs.
    pub spans: Page<Span, 512>,
    pub links: Page<Link, 32>,
    /// Sorted, disjoint runs over the exact input, including syntax markers.
    /// Oversized input styles only the bounded prefix retained in `text`.
    pub source_spans: Page<Span, 512>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Line {
    pub range: Range<usize>,
    /// Fenced or indented block code; inline code is described by spans only.
    pub code: bool,
    pub quote: bool,
    /// Zero for body text, otherwise CommonMark heading level 1 through 6.
    pub heading: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub marker: bool,
    pub heading: u8,
    pub quote: bool,
    pub link: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Link {
    pub range: Range<usize>,
    pub destination: Text<2048>,
}

/// Parse CommonMark with strikethrough and task lists. Every exceeded budget,
/// invalid parser offset or upstream unwind discards partial rendering and
/// returns literal text without links. The caller's source is never modified.
///
/// Inputs above 4 KiB are not parsed: a UTF-8 prefix and an ellipsis are shown.
/// Ordinary bounded inputs retain every source byte on literal fallback.
pub fn parse(source: &str) -> Document {
    if source.len() > MAX_SOURCE_BYTES {
        return literal(source);
    }
    // Older parser versions had reachable offset-iterator panics (#1129,
    // #1112). Retain isolation after upgrading: malformed input is untrusted.
    // Keep parser construction and iteration inside the boundary. Do not
    // replace the process-wide panic hook: concurrent callers must not race.
    std::panic::catch_unwind(|| render::parse(source))
        .ok()
        .flatten()
        .unwrap_or_else(|| literal(source))
}

fn literal(source: &str) -> Document {
    let mut end = source.len().min(MAX_SOURCE_BYTES);
    while !source.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let retained = source.get(..end).unwrap_or_default();
    let mut display = retained.to_owned();
    if end < source.len() {
        display.push('…');
    }
    let styles = vec![Style::default(); display.len()];
    let block_code = vec![false; display.len()];
    Document {
        text: Text::new(&display).unwrap_or_else(|_| empty_text()),
        lines: line_runs(&display, &styles, &block_code).unwrap_or_else(Page::empty),
        spans: plain_span(display.len()),
        links: Page::empty(),
        source_spans: plain_span(end),
    }
}

// Total fallbacks avoid assertions even if a future internal budget changes.
fn empty_text<const N: usize>() -> Text<N> {
    Text::new("").expect("empty text fits every byte bound")
}

fn plain_span<const N: usize>(end: usize) -> Page<Span, N> {
    let mut spans = Page::empty();
    if end != 0 {
        let _ = spans.try_push(Span {
            range: 0..end,
            style: Style::default(),
        });
    }
    spans
}

fn style_runs(bytes: &[Style]) -> Option<Page<Span, MAX_SPANS>> {
    let mut result = Page::empty();
    let mut start = 0;
    while let Some(style) = bytes.get(start).copied() {
        let end = bytes
            .get(start..)?
            .iter()
            .position(|next| *next != style)
            .map_or(bytes.len(), |offset| start + offset);
        result
            .try_push(Span {
                range: start..end,
                style,
            })
            .ok()?;
        start = end;
    }
    Some(result)
}

fn line_runs(text: &str, styles: &[Style], block_code: &[bool]) -> Option<Page<Line, MAX_LINES>> {
    let mut result = Page::empty();
    let mut start = 0;
    for (offset, character) in text.char_indices() {
        if character != '\n' && character != '\r' {
            continue;
        }
        // Treat CRLF as one line ending without dropping either source byte.
        if character == '\n' && offset > 0 && text.as_bytes().get(offset - 1) == Some(&b'\r') {
            start = offset + 1;
            continue;
        }
        push_line(&mut result, start..offset, styles, block_code)?;
        start = offset + character.len_utf8();
    }
    push_line(&mut result, start..text.len(), styles, block_code)?;
    Some(result)
}

fn push_line(
    lines: &mut Page<Line, MAX_LINES>,
    range: Range<usize>,
    styles: &[Style],
    block_code: &[bool],
) -> Option<()> {
    let mut line = Line {
        range: range.clone(),
        code: false,
        quote: false,
        heading: 0,
    };
    // Include a newline's style for empty code/quote lines.
    let end = range
        .end
        .max(range.start.saturating_add(1))
        .min(styles.len());
    line.code = block_code.get(range.start..end)?.iter().any(|code| *code);
    for style in styles.get(range.start..end)? {
        line.quote |= style.quote;
        line.heading = line.heading.max(style.heading);
    }
    lines.try_push(line).ok()
}

#[cfg(test)]
mod tests;
