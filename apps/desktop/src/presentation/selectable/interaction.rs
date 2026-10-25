use super::document::DISPLAY_BYTES;
use gpui::{ElementId, Pixels, Point, SharedString, px};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, PartialEq)]
pub(super) struct Identity {
    pub id: ElementId,
    pub source: SharedString,
    pub text: SharedString,
}

struct Press {
    ordinal: u16,
    origin: Point<Pixels>,
    cancelled: bool,
}

#[derive(Default)]
pub(super) struct SelectionState {
    pub target: Option<Identity>,
    pub anchor: usize,
    pub head: usize,
    pub dragging: bool,
    press: Option<Press>,
    key: Option<(Identity, u16, String)>,
    key_live: bool,
}

#[derive(Clone, Copy)]
pub(super) enum Edge {
    Left,
    Right,
    Start,
    End,
}

pub(super) fn boundary(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(offset, _)| offset)
        .chain(std::iter::once(text.len()))
        .take_while(|offset| *offset <= index)
        .last()
        .unwrap_or(0)
}

impl SelectionState {
    pub fn begin_frame(&mut self) {
        self.key_live = false;
    }
    pub fn rendered(&mut self, identity: &Identity) {
        self.key_live |= self
            .key
            .as_ref()
            .is_some_and(|(target, _, _)| target == identity);
    }
    pub fn finish_frame(&mut self) {
        if !self.key_live {
            self.cancel_key();
        }
    }
    pub fn matches(&self, identity: &Identity) -> bool {
        self.target.as_ref() == Some(identity)
    }
    pub fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    pub fn begin(&mut self, identity: Identity, index: usize, clicks: usize, shift: bool) {
        if identity.text.len() > DISPLAY_BYTES {
            return;
        }
        let index = boundary(&identity.text, index);
        if !shift || !self.matches(&identity) {
            self.anchor = index;
        }
        self.head = index;
        if clicks >= 3 {
            self.anchor = 0;
            self.head = identity.text.len();
        } else if clicks == 2 {
            let word = identity
                .text
                .unicode_word_indices()
                .find(|(start, word)| *start <= index && index < *start + word.len())
                .map(|(start, word)| start..start + word.len())
                .unwrap_or(index..index);
            self.anchor = word.start;
            self.head = word.end;
        }
        self.target = Some(identity);
        self.dragging = clicks == 1;
        self.press = None;
        self.key = None;
    }

    pub fn pointer_down(
        &mut self,
        identity: Identity,
        index: usize,
        clicks: usize,
        shift: bool,
        position: Point<Pixels>,
        link: Option<u16>,
    ) {
        let had_selection = !self.range().is_empty();
        self.begin(identity, index, clicks, shift);
        if clicks == 1 && !shift && !had_selection {
            self.press = link.map(|ordinal| Press {
                ordinal,
                origin: position,
                cancelled: false,
            });
        }
    }

    pub fn pointer_move(&mut self, index: usize, position: Point<Pixels>, left: bool) {
        if !left {
            self.cancel_pointer();
            return;
        }
        if !self.dragging {
            return;
        }
        if let Some(target) = &self.target {
            self.head = boundary(&target.text, index);
        }
        if let Some(press) = &mut self.press {
            press.cancelled |= self.head != self.anchor || moved(press.origin, position);
        }
    }

    pub fn pointer_up(
        &mut self,
        identity: &Identity,
        position: Point<Pixels>,
        link: Option<u16>,
        clicks: usize,
        shift: bool,
    ) -> Option<u16> {
        let press = self.press.take();
        let eligible = self.dragging
            && self.matches(identity)
            && self.range().is_empty()
            && clicks == 1
            && !shift;
        self.dragging = false;
        press
            .filter(|press| {
                eligible
                    && !press.cancelled
                    && !moved(press.origin, position)
                    && link == Some(press.ordinal)
            })
            .map(|press| press.ordinal)
    }

    pub fn cancel_pointer(&mut self) {
        self.dragging = false;
        self.press = None;
    }
    pub fn cancel_key(&mut self) {
        self.key = None;
    }

    pub fn key_down(
        &mut self,
        identity: &Identity,
        ordinal: u16,
        key: &str,
        held: bool,
        modified: bool,
    ) {
        if held {
            return;
        }
        self.key = (!modified && !self.dragging && self.range().is_empty())
            .then(|| (identity.clone(), ordinal, key.to_owned()));
    }
    pub fn key_up(&mut self, identity: &Identity, ordinal: u16, key: &str, modified: bool) -> bool {
        let armed = self.key.take();
        !modified
            && self.range().is_empty()
            && !self.dragging
            && armed.as_ref().is_some_and(|(target, link, down)| {
                target == identity && *link == ordinal && down == key
            })
    }

    pub fn copied<'a>(&self, identity: &'a Identity, permitted: bool) -> Option<&'a str> {
        (permitted && self.matches(identity) && !self.range().is_empty())
            .then(|| identity.text.get(self.range()))
            .flatten()
    }
    pub fn extend(&mut self, edge: Edge) {
        let Some(target) = &self.target else {
            return;
        };
        self.head = match edge {
            Edge::Left => target
                .text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .take_while(|offset| *offset < self.head)
                .last()
                .unwrap_or(0),
            Edge::Right => target
                .text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .find(|offset| *offset > self.head)
                .unwrap_or(target.text.len()),
            Edge::Start => 0,
            Edge::End => target.text.len(),
        };
        self.cancel_pointer();
        self.cancel_key();
    }
}

fn moved(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    (a.x - b.x).abs() > px(3.) || (a.y - b.y).abs() > px(3.)
}
