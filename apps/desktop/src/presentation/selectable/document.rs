//! Prepared display data is owned by the one selection entity. Cache eviction
//! affects preparation only; it never changes a mounted element's identity.
use gpui::SharedString;
use std::collections::VecDeque;
use talk_core::ui::markup::{self, Line, Link, Span};

pub(super) const SOURCE_BYTES: usize = 4096;
pub(super) const DISPLAY_BYTES: usize = 8192;
pub(super) const MAX_LINKS: usize = 32;
const CACHE_ENTRIES: usize = 96;
const CACHE_SOURCE_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub(super) struct Block {
    pub source: SharedString,
    pub text: SharedString,
    pub lines: Vec<Line>,
    pub spans: Vec<Span>,
    pub links: Vec<Link>,
}

impl Block {
    pub fn new(source: SharedString, rich: bool) -> Self {
        if rich {
            let document = markup::parse(&source);
            let mut lines = document.lines.as_slice().to_vec();
            if lines.is_empty() && !document.text.as_str().is_empty() {
                // The parser intentionally uses one literal block when a
                // bounded source exceeds its line budget. Never hide it.
                lines.push(Line {
                    range: 0..document.text.as_str().len(),
                    code: false,
                    quote: false,
                    heading: 0,
                });
            }
            Self {
                source,
                text: document.text.as_str().to_owned().into(),
                lines,
                spans: document.spans.as_slice().to_vec(),
                links: document.links.as_slice().to_vec(),
            }
        } else {
            Self {
                text: source.clone(),
                lines: vec![Line {
                    range: 0..source.len(),
                    code: false,
                    quote: false,
                    heading: 0,
                }],
                source,
                spans: Vec::new(),
                links: Vec::new(),
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Documents {
    entries: VecDeque<(Block, bool)>,
    source_bytes: usize,
}

impl Documents {
    pub fn begin_frame(&mut self) {
        for (_, live) in &mut self.entries {
            *live = false;
        }
    }

    pub fn finish_frame(&mut self) {
        self.entries.retain(|(_, live)| *live);
        self.source_bytes = self
            .entries
            .iter()
            .map(|(block, _)| block.source.len())
            .sum();
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.source_bytes = 0;
    }

    pub fn link_count(&self, source: &str) -> usize {
        self.entries
            .iter()
            .find(|(block, _)| block.source.as_ref() == source)
            .map_or(MAX_LINKS, |(block, _)| block.links.len())
    }

    pub fn prepare(&mut self, source: SharedString) -> Block {
        if let Some(index) = self
            .entries
            .iter()
            .position(|(block, _)| block.source == source)
            && let Some((block, _)) = self.entries.remove(index)
        {
            self.entries.push_back((block.clone(), true));
            return block;
        }
        // Admission precedes parsing/allocation. Callers bound the source too;
        // this guard keeps this owner safe if another producer is added.
        if source.len() > SOURCE_BYTES {
            return Block::new(SharedString::default(), false);
        }
        while self.entries.len() >= CACHE_ENTRIES
            || self.source_bytes + source.len() > CACHE_SOURCE_BYTES
        {
            let Some((old, _)) = self.entries.pop_front() else {
                break;
            };
            self.source_bytes -= old.source.len();
        }
        let block = Block::new(source, true);
        self.source_bytes += block.source.len();
        self.entries.push_back((block.clone(), true));
        block
    }

    #[cfg(test)]
    pub fn retained(&self) -> (usize, usize) {
        (self.entries.len(), self.source_bytes)
    }
}
