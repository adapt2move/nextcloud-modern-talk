use std::ops::Range;

use super::Style;

pub(super) struct Source<'a> {
    text: &'a str,
    pub styles: Vec<Style>,
    pub prose: Vec<bool>,
}

impl<'a> Source<'a> {
    pub fn new(text: &'a str) -> Self {
        Self {
            text,
            styles: vec![Style::default(); text.len()],
            prose: vec![false; text.len()],
        }
    }

    pub fn as_str(&self) -> &str {
        self.text
    }

    pub fn paint(&mut self, range: Range<usize>, style: Style, prose: bool) -> Option<()> {
        self.text.get(range.clone())?;
        self.styles.get_mut(range.clone())?.fill(style);
        self.prose.get_mut(range)?.fill(prose);
        Some(())
    }

    pub fn text(&mut self, range: Range<usize>, style: Style, prose: bool) -> Option<()> {
        self.paint(range.clone(), style, prose)?;
        let raw = self.text.get(range.clone())?;
        // Escapes are source syntax even though the parser emits their literal
        // punctuation. Iterate char boundaries; never scan into UTF-8 bytes.
        let mut characters = raw.char_indices();
        while let Some((offset, character)) = characters.next() {
            if character == '\\' {
                let mut next = characters.clone();
                if next
                    .next()
                    .is_some_and(|(_, value)| value.is_ascii_punctuation())
                {
                    if let Some(style) = self.styles.get_mut(range.start + offset) {
                        style.marker = true;
                    }
                    characters = next;
                }
            }
        }
        Some(())
    }

    pub fn inline_code(&mut self, range: Range<usize>, mut style: Style) -> Option<()> {
        style.code = true;
        self.paint(
            range.clone(),
            Style {
                marker: true,
                ..style
            },
            false,
        )?;
        let raw = self.text.get(range.clone())?;
        let opening = raw.bytes().take_while(|byte| *byte == b'`').count();
        let closing = raw.bytes().rev().take_while(|byte| *byte == b'`').count();
        if opening + closing <= raw.len() {
            self.paint(range.start + opening..range.end - closing, style, false)?;
        }
        Some(())
    }
}
