use std::ops::Range;

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

use super::source::Source;
use super::{
    Document, Link, MAX_DEPTH, MAX_EVENTS, MAX_LINKS, MAX_TEXT_BYTES, Page, Style, Text, line_runs,
    links, style_runs,
};

struct Frame {
    end: TagEnd,
    previous_style: Style,
    previous_blocked: usize,
    output_start: usize,
    source_range: Range<usize>,
    destination: Option<Text<2048>>,
    next_number: Option<u64>,
}

struct Builder<'a> {
    source: Source<'a>,
    text: String,
    styles: Vec<Style>,
    // Parallel display-byte mask under MAX_TEXT_BYTES. Inline code changes
    // Style::code, but only a CodeBlock parser frame marks whole code lines.
    block_code: Vec<bool>,
    prose: Vec<bool>,
    links: Vec<Link>,
    frames: Vec<Frame>,
    style: Style,
    blocked: usize,
    pending_breaks: usize,
}

pub(super) fn parse(source: &str) -> Option<Document> {
    // Deliberately enable only these extensions. In particular, wikilinks,
    // metadata, math, heading attributes and raw-HTML interpretation are absent.
    let options = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        source: Source::new(source),
        text: String::new(),
        styles: Vec::new(),
        block_code: Vec::new(),
        prose: Vec::new(),
        links: Vec::new(),
        frames: Vec::new(),
        style: Style::default(),
        blocked: 0,
        pending_breaks: 0,
    };
    let leading = source
        .chars()
        .take_while(|character| character.is_ascii_whitespace())
        .filter(|character| *character == '\n')
        .count();
    for _ in 0..leading {
        builder.append_raw("\n", false)?;
    }
    for (count, (event, range)) in Parser::new_ext(source, options)
        .into_offset_iter()
        .enumerate()
    {
        if count >= MAX_EVENTS {
            return None;
        }
        source.get(range.clone())?;
        builder.event(event, range, source)?;
    }
    if !builder.frames.is_empty() {
        return None;
    }
    if builder.text.is_empty() && !source.is_empty() {
        // Empty/definition-only Markdown still has readable literal content.
        return None;
    }
    // CommonMark discards exterior blank lines. Chat keeps their line breaks;
    // code's emitted terminal newline already counts towards this suffix.
    let trailing = trailing_breaks(source);
    if trailing > 0 {
        builder.pending_breaks = trailing;
        builder.flush_breaks()?;
    }
    builder.finish()
}

impl Builder<'_> {
    fn event(&mut self, event: Event<'_>, range: Range<usize>, original: &str) -> Option<()> {
        match event {
            Event::Start(tag) => self.start(tag, range)?,
            Event::End(end) => self.end(end, original)?,
            Event::Text(text) => {
                let prose = self.blocked == 0 && !self.style.code;
                if self.style.code {
                    self.source.paint(range, self.style, false)?;
                } else {
                    self.source.text(range, self.style, prose)?;
                }
                self.append(&text, prose)?;
            }
            Event::Code(text) => {
                let previous = self.style;
                self.style.code = true;
                self.source.inline_code(range, self.style)?;
                self.append(&text, false)?;
                self.style = previous;
            }
            Event::Html(text) | Event::InlineHtml(text) => {
                // Raw markup is visible data; even URLs in attributes do not
                // create remote requests or clickable HTML elements.
                self.source.paint(range, self.style, false)?;
                self.append(&text, false)?;
            }
            Event::SoftBreak | Event::HardBreak => {
                self.source.paint(
                    range,
                    Style {
                        marker: true,
                        ..self.style
                    },
                    false,
                )?;
                self.append("\n", false)?;
            }
            Event::TaskListMarker(checked) => {
                self.source.paint(
                    range,
                    Style {
                        marker: true,
                        ..self.style
                    },
                    false,
                )?;
                self.append(if checked { "[x] " } else { "[ ] " }, false)?;
            }
            Event::Rule => {
                self.boundary(1);
                self.source.paint(
                    range.clone(),
                    Style {
                        marker: true,
                        ..self.style
                    },
                    false,
                )?;
                self.append(original.get(range)?.trim_end_matches(['\r', '\n']), false)?;
                self.boundary(1);
            }
            // These events require extensions that are disabled above. If an
            // upstream parser adds another route, keep its original spelling.
            Event::InlineMath(_) | Event::DisplayMath(_) | Event::FootnoteReference(_) => {
                self.source.paint(range.clone(), self.style, false)?;
                self.append(original.get(range)?, false)?;
            }
        }
        Some(())
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) -> Option<()> {
        if self.frames.len() >= MAX_DEPTH {
            return None;
        }
        if matches!(tag, Tag::Link { .. } | Tag::Image { .. }) {
            self.flush_breaks()?;
        }
        let mut frame = Frame {
            end: tag.to_end(),
            previous_style: self.style,
            previous_blocked: self.blocked,
            output_start: self.text.len(),
            source_range: range.clone(),
            destination: None,
            next_number: None,
        };
        match tag {
            Tag::Heading { level, .. } => {
                self.boundary(1);
                self.style.heading = level as u8;
            }
            Tag::BlockQuote(_) => {
                self.boundary(1);
                self.style.quote = true;
            }
            Tag::CodeBlock(_) => {
                self.boundary(1);
                self.style.code = true;
            }
            Tag::List(number) => {
                self.boundary(1);
                frame.next_number = number;
            }
            Tag::Item => {
                self.boundary(1);
                let depth = self
                    .frames
                    .iter()
                    .filter(|frame| matches!(frame.end, TagEnd::List(_)))
                    .count();
                let mut prefix = "  ".repeat(depth.saturating_sub(1));
                let list = self
                    .frames
                    .iter_mut()
                    .rev()
                    .find(|frame| matches!(frame.end, TagEnd::List(_)))?;
                if let Some(number) = list.next_number {
                    prefix.push_str(&format!("{number}. "));
                    list.next_number = Some(number.saturating_add(1));
                } else {
                    prefix.push_str("• ");
                }
                self.append(&prefix, false)?;
            }
            Tag::Emphasis => self.style.italic = true,
            Tag::Strong => self.style.bold = true,
            Tag::Strikethrough => self.style.strike = true,
            Tag::Link { dest_url, .. } => {
                if self.blocked == 0 {
                    frame.destination = links::web_destination(&dest_url);
                }
                self.style.link = frame.destination.is_some();
                self.blocked += 1;
            }
            Tag::Image { .. } => {
                self.style.link = false;
                self.blocked += 1;
            }
            _ => {}
        }
        // The parser gives a whole-element range for Start tags. Paint it as
        // syntax first; its leaf events reveal original content in place.
        self.source.paint(
            range,
            Style {
                marker: true,
                ..self.style
            },
            false,
        )?;
        self.frames.push(frame);
        Some(())
    }

    fn end(&mut self, end: TagEnd, original: &str) -> Option<()> {
        let frame = self.frames.pop()?;
        if frame.end != end {
            return None;
        }
        match end {
            TagEnd::Link => {
                if let Some(destination) = frame.destination {
                    let range = frame.output_start..self.text.len();
                    if !range.is_empty() {
                        if self.links.len() == MAX_LINKS {
                            return None;
                        }
                        self.links.push(Link { range, destination });
                    }
                }
            }
            TagEnd::Image if frame.output_start == self.text.len() => {
                // An image without alt text must not disappear or load.
                self.append(original.get(frame.source_range)?, false)?;
            }
            TagEnd::Paragraph => {
                self.boundary(
                    if self.frames.iter().any(|frame| frame.end == TagEnd::Item) {
                        1
                    } else {
                        2
                    },
                );
            }
            TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::Item
            | TagEnd::List(_)
            | TagEnd::HtmlBlock => self.boundary(1),
            _ => {}
        }
        self.style = frame.previous_style;
        self.blocked = frame.previous_blocked;
        Some(())
    }

    fn boundary(&mut self, breaks: usize) {
        self.pending_breaks = self.pending_breaks.max(breaks);
    }

    fn flush_breaks(&mut self) -> Option<()> {
        let desired = std::mem::take(&mut self.pending_breaks);
        if self.text.is_empty() {
            return Some(());
        }
        let existing = trailing_breaks(&self.text);
        for _ in existing..desired {
            // Deferred block separators are outside the incoming code block;
            // real blank code lines arrive in its Text event instead.
            self.append_bytes("\n", false, false)?;
        }
        Some(())
    }

    fn append(&mut self, text: &str, prose: bool) -> Option<()> {
        if !text.is_empty() {
            self.flush_breaks()?;
            self.append_raw(text, prose)?;
        }
        Some(())
    }

    fn append_raw(&mut self, text: &str, prose: bool) -> Option<()> {
        let block_code = self
            .frames
            .iter()
            .any(|frame| frame.end == TagEnd::CodeBlock);
        self.append_bytes(text, prose, block_code)
    }

    fn append_bytes(&mut self, text: &str, prose: bool, block_code: bool) -> Option<()> {
        let length = self.text.len().checked_add(text.len())?;
        if length > MAX_TEXT_BYTES {
            return None;
        }
        self.text.push_str(text);
        self.styles.resize(length, self.style);
        self.block_code.resize(length, block_code);
        self.prose.resize(length, prose);
        Some(())
    }

    fn finish(mut self) -> Option<Document> {
        for recognized in links::recognize(&self.text, &self.prose)? {
            if self.links.len() == MAX_LINKS {
                return None;
            }
            links::mark_link(&mut self.styles, recognized.range.clone())?;
            self.links.push(Link {
                range: recognized.range,
                destination: recognized.destination,
            });
        }
        for recognized in links::recognize(self.source_text(), &self.source.prose)? {
            links::mark_link(&mut self.source.styles, recognized.range)?;
        }
        self.links.sort_by_key(|link| link.range.start);
        let mut links = Page::empty();
        let mut previous_end = 0;
        for link in self.links {
            if link.range.start < previous_end {
                return None;
            }
            previous_end = link.range.end;
            links.try_push(link).ok()?;
        }
        Some(Document {
            text: Text::new(&self.text).ok()?,
            lines: line_runs(&self.text, &self.styles, &self.block_code)?,
            spans: style_runs(&self.styles)?,
            links,
            source_spans: style_runs(&self.source.styles)?,
        })
    }

    fn source_text(&self) -> &str {
        self.source.as_str()
    }
}

fn trailing_breaks(text: &str) -> usize {
    let mut breaks = 0;
    let mut previous_lf = false;
    for character in text.chars().rev() {
        match character {
            '\n' => {
                breaks += 1;
                previous_lf = true;
            }
            '\r' => {
                if !previous_lf {
                    breaks += 1;
                }
                previous_lf = false;
            }
            _ => break,
        }
    }
    breaks
}
