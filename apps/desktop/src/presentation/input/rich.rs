//! Composer document: native offsets address `text`, never serialized Markdown.
//!
//! Admission is transactional. Text/byte attributes are capped at 4096 bytes,
//! blocks at 256, links at 32, code groups at 64, and import nesting at 16.
//! Canonical wire text and display spans are rebuilt only after document edits;
//! selection changes neither serialize nor parse. History retains 16 documents
//! per direction and one composition baseline (owned by `edit`).
use gpui::SharedString;
use std::ops::Range;
use talk_core::ui::markup::{Span, Style};

mod codec;
#[cfg(test)]
mod tests;

pub(super) const MAX_BYTES: usize = 4096;
const MAX_BLOCKS: usize = 256;
const MAX_LINKS: usize = 32;
const MAX_CODES: usize = 64;
const BOLD: u8 = 1;
const ITALIC: u8 = 2;
const STRIKE: u8 = 4;
const CODE: u8 = 8;
// Escaped punctuation is literal even when a later edit balances nearby syntax.
const LITERAL: u8 = 16;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Inline {
    flags: u8,
    link: u8, // 0 = none, otherwise one-based into links
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct List {
    depth: u8,
    number: Option<u32>,
    task: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Block {
    heading: u8,
    quote: u8,
    list: Option<List>,
    code: Option<u8>,
}

impl Block {
    fn continuation(self) -> Self {
        Self {
            heading: 0,
            list: self.list.map(|list| List {
                number: list.number.map(|n| n.saturating_add(1)),
                task: list.task.map(|_| false),
                ..list
            }),
            ..self
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Format {
    Bold,
    Italic,
    Code,
}
impl Format {
    fn flag(self) -> u8 {
        match self {
            Self::Bold => BOLD,
            Self::Italic => ITALIC,
            Self::Code => CODE,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Decoration {
    pub label: Option<SharedString>,
    pub quote: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Document {
    pub text: SharedString,
    marks: Vec<Inline>,
    blocks: Vec<Block>,
    links: Vec<SharedString>,
    codes: Vec<SharedString>,
    pub wire: SharedString,
    pub spans: Vec<Span>,
    pub decorations: Vec<Decoration>,
    typing: Option<Inline>,
}

impl Document {
    pub fn import(source: &str, limit: usize) -> Option<Self> {
        let limit = limit.min(MAX_BYTES);
        if source.len() > limit {
            return None;
        }
        if source.contains('\r') {
            let normalized = source.replace("\r\n", "\n").replace('\r', "\n");
            codec::import(&normalized, limit)
        } else {
            codec::import(source, limit)
        }
    }

    pub fn clear_typing(&mut self) {
        self.typing = None;
    }

    pub fn in_code(&self, offset: usize) -> bool {
        self.blocks[self.row(offset)].code.is_some()
            || self.inherited(&(offset..offset)).flags & CODE != 0
    }

    fn row(&self, offset: usize) -> usize {
        self.text[..offset].bytes().filter(|c| *c == b'\n').count()
    }

    fn inherited(&self, range: &Range<usize>) -> Inline {
        if let Some(typing) = self.typing {
            return typing;
        }
        if !range.is_empty() {
            return self.marks.get(range.start).copied().unwrap_or_default();
        }
        let mut mark = if range.start > 0 && self.text.as_bytes()[range.start - 1] != b'\n' {
            self.marks.get(range.start - 1).copied().unwrap_or_default()
        } else {
            self.marks.get(range.start).copied().unwrap_or_default()
        };
        // Literalness belongs to characters, rather than subsequent typing.
        mark.flags &= !LITERAL;
        if self
            .marks
            .get(range.start)
            .is_none_or(|next| next.link != mark.link)
        {
            mark.link = 0;
        }
        mark
    }

    /// Edits use visible UTF-8 boundaries. Preedit updates call with conversion
    /// disabled; only the final committed insertion may consume syntax.
    fn splice(
        &self,
        range: Range<usize>,
        text: &str,
        limit: usize,
    ) -> Option<(Self, Range<usize>)> {
        if !super::edit::replacement_fits(&self.text, &range, text, limit.min(MAX_BYTES), true) {
            return None;
        }
        let first = self.row(range.start);
        let last = self.row(range.end);
        let new_rows = text.bytes().filter(|c| *c == b'\n').count();
        if self.blocks.len() - (last - first) + new_rows > MAX_BLOCKS {
            return None;
        }
        let mut next = self.clone();
        let mark = self.inherited(&range);
        let mut content = self.text.to_string();
        content.replace_range(range.clone(), text);
        next.text = content.into();
        next.marks
            .splice(range.clone(), std::iter::repeat_n(mark, text.len()));
        let mut block = self.blocks[first];
        let mut inserted_blocks = vec![block];
        for _ in text.bytes().filter(|c| *c == b'\n') {
            block = block.continuation();
            inserted_blocks.push(block);
        }
        next.blocks.splice(first..=last, inserted_blocks);
        if next.text.is_empty() && !range.is_empty() {
            next.blocks[0] = Block::default();
            next.typing = None;
        }
        let cursor = range.start + text.len();
        Some((next, cursor..cursor))
    }

    pub fn replace(
        &self,
        range: Range<usize>,
        text: &str,
        convert: bool,
        limit: usize,
    ) -> Option<(Self, Range<usize>)> {
        let (mut next, selection) = self.splice(range, text, limit)?;
        let selection = if convert && !text.is_empty() {
            next.convert(selection)?
        } else {
            selection
        };
        next.finish(limit)?;
        Some((next, selection))
    }

    /// Paste imports Markdown as a document, except inside literal code.
    pub fn paste(
        &self,
        range: Range<usize>,
        source: &str,
        limit: usize,
    ) -> Option<(Self, Range<usize>)> {
        if self.in_code(range.start) {
            return self.replace(range, source, false, limit);
        }
        let pasted = Self::import(source, limit)?;
        let (mut next, selection) = self.splice(range.clone(), &pasted.text, limit)?;
        if next.links.len() + pasted.links.len() > MAX_LINKS
            || next.codes.len() + pasted.codes.len() > MAX_CODES
        {
            return None;
        }
        let link_base = next.links.len() as u8;
        let code_base = next.codes.len() as u8;
        next.links.extend(pasted.links);
        next.codes.extend(pasted.codes);
        let mut marks = pasted.marks;
        for mark in &mut marks {
            if mark.link > 0 {
                mark.link += link_base;
            }
        }
        next.marks
            .splice(range.start..range.start + pasted.text.len(), marks);
        let row = self.row(range.start);
        for (i, mut block) in pasted.blocks.into_iter().enumerate() {
            if let Some(code) = &mut block.code {
                *code += code_base;
            }
            // Inline paste into an existing block retains its block semantics.
            if (i > 0 || self.text[..range.start].ends_with('\n') || range.start == 0)
                && let Some(target) = next.blocks.get_mut(row + i)
            {
                *target = block;
            }
        }
        next.typing = Some(Inline::default());
        next.finish(limit)?;
        Some((next, selection))
    }

    pub fn toggle(&self, range: Range<usize>, format: Format, limit: usize) -> Option<Self> {
        self.text.get(range.clone())?;
        let mut next = self.clone();
        let flag = format.flag();
        if range.is_empty() {
            let mut mark = self.inherited(&range);
            mark.flags ^= flag;
            next.typing = Some(mark);
        } else {
            let remove = self.marks[range.clone()]
                .iter()
                .all(|m| m.flags & flag != 0);
            for mark in &mut next.marks[range] {
                if remove {
                    mark.flags &= !flag;
                } else {
                    mark.flags |= flag;
                }
            }
            next.typing = None;
        }
        next.finish(limit)?;
        Some(next)
    }

    /// Newline continues lists/quotes/code, resets headings, and leaves an
    /// empty list/quote block without inserting a phantom empty item.
    pub fn newline(&self, range: Range<usize>, limit: usize) -> Option<(Self, Range<usize>)> {
        let row = self.row(range.start);
        let start = self.text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[range.end..]
            .find('\n')
            .map_or(self.text.len(), |i| range.end + i);
        let block = self.blocks[row];
        if range.is_empty() && start == end && (block.list.is_some() || block.quote > 0) {
            let mut next = self.clone();
            next.blocks[row] = Block::default();
            next.typing = None;
            next.finish(limit)?;
            return Some((next, range));
        }
        let (mut next, selection) = self.splice(range, "\n", limit)?;
        next.typing = None;
        next.finish(limit)?;
        Some((next, selection))
    }

    pub fn leave_empty_block(&self, cursor: usize, limit: usize) -> Option<Self> {
        let row = self.row(cursor);
        let start = self.text[..cursor].rfind('\n').map_or(0, |i| i + 1);
        if start != cursor || self.blocks[row] == Block::default() {
            return None;
        }
        let mut next = self.clone();
        next.blocks[row] = Block::default();
        next.typing = None;
        next.finish(limit)?;
        Some(next)
    }

    pub fn commit_preedit(
        &self,
        selection: Range<usize>,
        limit: usize,
    ) -> Option<(Self, Range<usize>)> {
        let mut next = self.clone();
        let selection = next.convert(selection)?;
        next.finish(limit)?;
        Some((next, selection))
    }

    fn convert(&mut self, selection: Range<usize>) -> Option<Range<usize>> {
        let row = self.row(selection.end);
        if self.blocks[row].code.is_some() {
            return Some(selection);
        }
        let start = self.text[..selection.end].rfind('\n').map_or(0, |i| i + 1);
        let end = self.text[selection.end..]
            .find('\n')
            .map_or(self.text.len(), |i| selection.end + i);
        // Syntax shortcuts require actual editable markers. Imported escapes
        // and existing code can display the same prefix without granting it
        // authority to change the block when a later space is entered.
        if selection.is_empty()
            && selection.end == end
            && self.text[start..end].ends_with(' ')
            && self.marks[start..end]
                .iter()
                .all(|mark| mark.flags & (LITERAL | CODE) == 0)
        {
            let prefix = &self.text[start..end];
            let block = if self.blocks[row] == Block::default() {
                codec::shortcut(prefix, &mut self.codes)
            } else if let Some(list) = self.blocks[row].list.filter(|list| list.task.is_none()) {
                let checked = match prefix {
                    "[ ] " => Some(false),
                    "[x] " | "[X] " => Some(true),
                    _ => None,
                };
                checked.map(|checked| Block {
                    list: Some(List {
                        task: Some(checked),
                        ..list
                    }),
                    ..self.blocks[row]
                })
            } else {
                None
            };
            if let Some(block) = block {
                let mut content = self.text.to_string();
                content.replace_range(start..end, "");
                self.text = content.into();
                self.marks.drain(start..end);
                self.blocks[row] = block;
                self.typing = Some(Inline::default());
                return Some(start..start);
            }
        }
        let parsed = codec::inline(
            &self.text[start..end],
            &self.marks[start..end],
            &mut self.links,
            true,
        )?;
        if parsed.text == self.text[start..end] && parsed.marks == self.marks[start..end] {
            return Some(selection);
        }
        let map = |offset: usize| {
            if offset < start {
                offset
            } else if offset <= end {
                start + parsed.map[offset - start]
            } else {
                offset - (end - start) + parsed.text.len()
            }
        };
        let selection = map(selection.start)..map(selection.end);
        let mut content = self.text.to_string();
        content.replace_range(start..end, &parsed.text);
        self.text = content.into();
        self.marks.splice(start..end, parsed.marks);
        self.typing = Some(Inline::default());
        Some(selection)
    }

    fn finish(&mut self, limit: usize) -> Option<()> {
        if self.text.len() > limit.min(MAX_BYTES)
            || self.blocks.len() > MAX_BLOCKS
            || self.marks.len() != self.text.len()
            || self.blocks.len() != self.text.bytes().filter(|c| *c == b'\n').count() + 1
        {
            return None;
        }
        // Compact tables after deletion; repeated paste cannot accumulate dead
        // destinations/code groups and eventually exhaust an invisible cache.
        codec::compact(self);
        self.wire = codec::export(self, limit.min(MAX_BYTES))?.into();
        self.decorations.clear();
        for block in &self.blocks {
            let label = block.list.map(|list| {
                if let Some(checked) = list.task {
                    (if checked { "☑" } else { "☐" }).into()
                } else if let Some(number) = list.number {
                    format!("{number}.").into()
                } else {
                    "•".into()
                }
            });
            self.decorations.push(Decoration {
                label,
                quote: block.quote > 0,
                code: block.code.is_some(),
            });
        }
        self.spans.clear();
        let mut row = 0;
        for (offset, ch) in self.text.char_indices() {
            let mark = self.marks[offset];
            let block = self.blocks[row];
            let style = Style {
                bold: mark.flags & BOLD != 0,
                italic: mark.flags & ITALIC != 0,
                strike: mark.flags & STRIKE != 0,
                code: mark.flags & CODE != 0 || block.code.is_some(),
                heading: block.heading,
                quote: block.quote > 0,
                link: mark.link > 0,
                marker: false,
            };
            let end = offset + ch.len_utf8();
            if let Some(last) = self.spans.last_mut()
                && last.style == style
            {
                last.range.end = end;
            } else {
                self.spans.push(Span {
                    range: offset..end,
                    style,
                });
            }
            if ch == '\n' {
                row += 1;
            }
        }
        Some(())
    }
}
