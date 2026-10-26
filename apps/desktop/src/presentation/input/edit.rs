// Split from the GPUI 0.2.2 input adaptation; see ../input.rs and
// apps/desktop/assets/GPUI-LICENSE-APACHE for attribution and license.
//! Pure editing transitions shared by native insertion, composition and tests.
use std::ops::Range;

use gpui::SharedString;
use unicode_segmentation::UnicodeSegmentation;

/// Native mutation identity is independent of undo history and composition's
/// committed baseline. Exhaustion rejects before applying a candidate, so an
/// old receipt can never regain authority through wrapping or saturation.
#[derive(Default)]
pub(super) struct EditGeneration(u64);
impl EditGeneration {
    pub fn value(&self) -> u64 {
        self.0
    }
    pub fn available(&self) -> bool {
        self.0.checked_add(1).is_some()
    }
    pub fn admit<T>(&mut self, candidate: Option<T>) -> Option<T> {
        let candidate = candidate?;
        let generation = self.0.checked_add(1)?;
        self.0 = generation;
        Some(candidate)
    }
}

/// Persisted drafts may exceed a newly observed server limit; persistence keeps
/// them intact while every submission checks the current canonical wire size.
pub(super) fn submitted_text(value: &str, composing: bool, limit: usize) -> Option<&str> {
    (!composing && value.len() <= limit).then_some(value)
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct EditSnapshot {
    pub text: SharedString,
    pub selection: Range<usize>,
    pub reversed: bool,
    pub rich: Option<super::rich::Document>,
}

impl EditSnapshot {
    pub fn value(&self) -> &str {
        self.rich
            .as_ref()
            .map_or(self.text.as_ref(), |doc| doc.wire.as_ref())
    }
}

/// At most 16 snapshots in each direction and one bounded preedit baseline.
/// A composition is one edit, even if the platform updates its text repeatedly.
#[derive(Default)]
pub(super) struct EditHistory {
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
    composition: Option<EditSnapshot>,
}

impl EditHistory {
    fn push(list: &mut Vec<EditSnapshot>, state: EditSnapshot) {
        if list.len() == 16 {
            list.remove(0);
        }
        list.push(state);
    }

    pub fn record(&mut self, state: EditSnapshot) {
        Self::push(&mut self.undo, state);
        self.redo.clear();
    }

    pub fn begin_composition(&mut self, state: EditSnapshot) {
        self.composition.get_or_insert(state);
    }

    pub fn finish_composition(&mut self, text: &str) {
        if let Some(before) = self.composition.take()
            && before.value() != text
        {
            self.record(before);
        }
    }

    pub fn committed_value<'a>(&'a self, current: &'a str) -> &'a str {
        self.composition
            .as_ref()
            .map_or(current, EditSnapshot::value)
    }

    pub fn travel(&mut self, current: EditSnapshot, redo: bool) -> Option<EditSnapshot> {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let previous = from.pop()?;
        Self::push(to, current);
        Some(previous)
    }
}

pub(super) enum Insertion<'a> {
    Commit(&'a str),
    Paste(&'a str),
    Literal(&'a str),
    Newline,
    Mark {
        text: &'a str,
        selection_utf16: Option<Range<usize>>,
    },
}

pub(super) struct Replacement {
    pub state: EditSnapshot,
    pub marked: Option<Range<usize>>,
}

/// The platform's explicit document range wins over marked text and selection.
/// Selection in setMarkedText is relative to the replacement, never the document.
pub(super) fn replacement(
    current: &EditSnapshot,
    marked: Option<&Range<usize>>,
    range_utf16: Option<Range<usize>>,
    insertion: Insertion<'_>,
    limit: usize,
    multiline: bool,
) -> Option<Replacement> {
    let range = range_utf16
        .map(|r| range_from_utf16(&current.text, r))
        .or_else(|| marked.cloned())
        .unwrap_or_else(|| current.selection.clone());
    let paste = matches!(insertion, Insertion::Paste(_));
    let literal = matches!(insertion, Insertion::Literal(_));
    let newline = matches!(insertion, Insertion::Newline);
    let (text, selection, mark) = match insertion {
        Insertion::Commit(text) | Insertion::Paste(text) | Insertion::Literal(text) => {
            (text, None, false)
        }
        Insertion::Newline => ("\n", None, false),
        Insertion::Mark {
            text,
            selection_utf16,
        } => (text, selection_utf16, true),
    };
    if !replacement_fits(&current.text, &range, text, limit, multiline) {
        return None;
    }
    let selected = selection.map_or(text.len()..text.len(), |r| range_from_utf16(text, r));
    let (content, rich, committed_selection) = if let Some(document) = &current.rich {
        let (document, selected) = if paste {
            document.paste(range.clone(), text, limit)?
        } else if newline {
            document.newline(range.clone(), limit)?
        } else {
            document.replace(range.clone(), text, !mark && !literal, limit)?
        };
        (document.text.clone(), Some(document), selected)
    } else {
        let content = format!(
            "{}{}{}",
            &current.text[..range.start],
            text,
            &current.text[range.end..]
        );
        (
            content.into(),
            None,
            range.start + text.len()..range.start + text.len(),
        )
    };
    Some(Replacement {
        state: EditSnapshot {
            text: content,
            selection: if mark {
                range.start + selected.start..range.start + selected.end
            } else {
                committed_selection
            },
            reversed: false,
            rich,
        },
        marked: (mark && !text.is_empty()).then_some(range.start..range.start + text.len()),
    })
}

pub(super) fn from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= offset {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}

pub(super) fn to_utf16(text: &str, offset: usize) -> usize {
    text.char_indices()
        .take_while(|(i, _)| *i < offset)
        .map(|(_, c)| c.len_utf16())
        .sum()
}

pub(super) fn range_from_utf16(text: &str, range: Range<usize>) -> Range<usize> {
    let start = range.start.min(range.end);
    let end = range.start.max(range.end);
    let mut start_byte = from_utf16(text, start);
    // Expand a nonempty range around an indivisible surrogate pair. A collapsed
    // insertion remains collapsed, rounded after the scalar like a native caret.
    if start != end && to_utf16(text, start_byte) > start {
        start_byte = text[..start_byte]
            .char_indices()
            .next_back()
            .map_or(0, |(i, _)| i);
    }
    start_byte..from_utf16(text, end)
}

pub(super) fn replacement_fits(
    current: &str,
    range: &Range<usize>,
    text: &str,
    limit: usize,
    multiline: bool,
) -> bool {
    range.start <= range.end
        && range.end <= current.len()
        && current.is_char_boundary(range.start)
        && current.is_char_boundary(range.end)
        && text.len() <= limit
        && current.len() - (range.end - range.start) <= limit - text.len()
        && !text.contains(['\r', '\0'])
        && (multiline || !text.contains('\n'))
}

pub(super) fn word_boundary(text: &str, offset: usize, forward: bool) -> usize {
    if forward {
        text.unicode_word_indices()
            .map(|(i, word)| i + word.len())
            .find(|end| *end > offset)
            .unwrap_or(text.len())
    } else {
        text.unicode_word_indices()
            .rev()
            .find_map(|(i, _)| (i < offset).then_some(i))
            .unwrap_or(0)
    }
}

/// One bullet per grapheme, with explicit source/display byte mapping. Native
/// selections keep their original offsets; UTF-8 byte counts are not glyph counts.
pub(super) struct ConcealedText {
    pub text: SharedString,
    boundaries: Vec<(usize, usize)>,
}

impl ConcealedText {
    pub fn new(text: &str) -> Self {
        let mut boundaries: Vec<_> = text
            .grapheme_indices(true)
            .enumerate()
            .map(|(i, (source, _))| (source, i * '•'.len_utf8()))
            .collect();
        let count = boundaries.len();
        boundaries.push((text.len(), count * '•'.len_utf8()));
        Self {
            text: "•".repeat(count).into(),
            boundaries,
        }
    }
    pub fn display_offset(&self, source: usize) -> usize {
        self.boundaries
            .iter()
            .rev()
            .find(|(s, _)| *s <= source)
            .map_or(0, |(_, d)| *d)
    }
    pub fn source_offset(&self, display: usize) -> usize {
        self.boundaries
            .iter()
            .rev()
            .find(|(_, d)| *d <= display)
            .map_or(0, |(s, _)| *s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str, selection: Range<usize>) -> EditSnapshot {
        EditSnapshot {
            text: text.to_owned().into(),
            selection,
            reversed: true,
            rich: None,
        }
    }

    #[test]
    fn generation_distinguishes_preedit_from_the_unchanged_committed_draft_and_undo() {
        let before = snapshot("draft", 5..5);
        let mut history = EditHistory::default();
        let mut generation = EditGeneration::default();
        let receipt_generation = generation.value();
        let mut current = before.clone();
        let mut marked = None;
        for text in ["n", "ni", "ni"] {
            let previous_generation = generation.value();
            let next = generation
                .admit(replacement(
                    &current,
                    marked.as_ref(),
                    None,
                    Insertion::Mark {
                        text,
                        selection_utf16: None,
                    },
                    32,
                    true,
                ))
                .unwrap();
            history.begin_composition(current);
            current = next.state;
            marked = next.marked;
            assert!(generation.value() > previous_generation);
            assert_ne!(generation.value(), receipt_generation);
            assert_eq!(history.committed_value(&current.text), before.text.as_ref());
        }
        let previous_generation = generation.value();
        assert!(
            generation
                .admit(replacement(
                    &current,
                    marked.as_ref(),
                    None,
                    Insertion::Commit(&"x".repeat(33)),
                    32,
                    true
                ))
                .is_none()
        );
        assert_eq!(generation.value(), previous_generation);
        let committed = generation
            .admit(replacement(
                &current,
                marked.as_ref(),
                None,
                Insertion::Commit("你"),
                32,
                true,
            ))
            .unwrap()
            .state;
        history.finish_composition(committed.value());
        let previous_generation = generation.value();
        let restored = generation.admit(history.travel(committed, false)).unwrap();
        assert_eq!(restored, before);
        assert!(generation.value() > previous_generation);
    }

    #[test]
    fn exhausted_generation_rejects_edits_loads_and_resets_without_reusing_receipt_authority() {
        let mut generation = EditGeneration(u64::MAX - 1);
        let before = snapshot("draft", 5..5);
        let mut current = generation.admit(Some(before.clone())).unwrap();
        let receipt_generation = generation.value();
        assert_eq!(receipt_generation, u64::MAX);
        assert!(!generation.available());
        if let Some(next) = generation.admit(replacement(
            &current,
            None,
            None,
            Insertion::Commit("!"),
            32,
            true,
        )) {
            current = next.state;
        }
        assert_eq!(current, before);
        assert!(
            generation
                .admit(Some(snapshot("other context", 13..13)))
                .is_none()
        );
        assert!(generation.admit(Some(snapshot("", 0..0))).is_none());
        assert_eq!(generation.value(), receipt_generation);
        assert_eq!(current, before);
    }

    #[test]
    fn submissions_check_canonical_bytes_under_the_current_ceiling_without_discarding_drafts() {
        let doc = crate::presentation::input::rich::Document::import("**Ä🧭**", 4096).unwrap();
        assert!(doc.wire.len() > doc.text.len());
        let original = doc.clone();
        assert_eq!(
            submitted_text(&doc.wire, false, doc.wire.len()),
            Some(doc.wire.as_ref())
        );
        assert!(submitted_text(&doc.wire, false, doc.text.len()).is_none());
        assert!(submitted_text(&doc.wire, true, 4096).is_none());
        assert!(submitted_text(&doc.wire, false, 0).is_none());
        assert_eq!(doc, original);
        assert_eq!(
            submitted_text(&doc.wire, false, 4096),
            Some(doc.wire.as_ref())
        );
    }

    #[test]
    fn ime_updates_are_relative_and_commit_is_one_undoable_edit() {
        let before = snapshot("A🧭Z", 1..5);
        let mut history = EditHistory::default();
        let mut state = before.clone();
        let mut marked = None;
        for text in ["n", "ni", "你🧭"] {
            let next = replacement(
                &state,
                marked.as_ref(),
                None,
                Insertion::Mark {
                    text,
                    selection_utf16: Some(1..3),
                },
                32,
                true,
            )
            .unwrap();
            history.begin_composition(state);
            state = next.state;
            marked = next.marked;
            assert_eq!(history.committed_value(&state.text), before.text.as_ref());
        }
        assert_eq!(state.text.as_ref(), "A你🧭Z");
        assert_eq!(state.selection, 4..8);
        assert!(!state.reversed);
        let next = replacement(
            &state,
            marked.as_ref(),
            None,
            Insertion::Commit("你"),
            32,
            true,
        )
        .unwrap();
        state = next.state;
        assert_eq!(state.text.as_ref(), "A你Z");
        assert!(next.marked.is_none());
        history.finish_composition(&state.text);
        assert_eq!(history.committed_value(&state.text), "A你Z");
        let restored = history.travel(state.clone(), false).unwrap();
        assert_eq!(restored, before);
        assert!(history.travel(restored.clone(), false).is_none());
        assert_eq!(history.travel(restored, true).unwrap(), state);
    }

    #[test]
    fn explicit_utf16_replacement_wins_and_partial_surrogates_expand() {
        let state = snapshot("A🧭BC", 6..7);
        let next = replacement(
            &state,
            Some(&(5..6)),
            Some(2..3),
            Insertion::Commit("文"),
            32,
            true,
        )
        .unwrap();
        assert_eq!(next.state.text.as_ref(), "A文BC");
        assert_eq!(next.state.selection, 4..4);
        assert_eq!(range_from_utf16("A🧭B", 2..2), 5..5);
        assert_eq!(range_from_utf16("A🧭B", Range { start: 3, end: 2 }), 1..5);
        assert_eq!(range_from_utf16("A🧭B", 99..usize::MAX), 6..6);
    }

    #[test]
    fn rejected_composition_preserves_preedit_and_history_and_cancellation_is_noop() {
        let before = snapshot("AB", 1..1);
        let mut history = EditHistory::default();
        let marked = replacement(
            &before,
            None,
            None,
            Insertion::Mark {
                text: "文",
                selection_utf16: None,
            },
            5,
            false,
        )
        .unwrap();
        history.begin_composition(before.clone());
        assert!(
            replacement(
                &marked.state,
                marked.marked.as_ref(),
                None,
                Insertion::Mark {
                    text: "文🧭",
                    selection_utf16: None
                },
                5,
                false
            )
            .is_none()
        );
        assert_eq!(history.committed_value(&marked.state.text), "AB");
        let cancelled = replacement(
            &marked.state,
            marked.marked.as_ref(),
            None,
            Insertion::Mark {
                text: "",
                selection_utf16: None,
            },
            5,
            false,
        )
        .unwrap();
        history.finish_composition(&cancelled.state.text);
        assert_eq!(cancelled.state.text.as_ref(), "AB");
        assert!(cancelled.marked.is_none());
        assert!(history.travel(cancelled.state, false).is_none());
    }

    #[test]
    fn unmark_records_baseline_and_new_edit_retires_redo() {
        let before = snapshot("text", 4..4);
        let after = snapshot("texté", 6..6);
        let mut history = EditHistory::default();
        history.begin_composition(before.clone());
        history.finish_composition(&after.text);
        let restored = history.travel(after.clone(), false).unwrap();
        assert_eq!(restored, before);
        history.record(restored);
        assert!(history.travel(after, true).is_none());
    }

    #[test]
    fn deleting_a_cluster_restores_the_original_caret_on_undo() {
        let before = snapshot("e\u{301}🧭", 7..7);
        let next = replacement(&before, None, Some(2..4), Insertion::Commit(""), 32, true).unwrap();
        assert_eq!(next.state.text.as_ref(), "e\u{301}");
        assert_eq!(next.state.selection, 3..3);
        let mut history = EditHistory::default();
        history.record(before.clone());
        assert_eq!(history.travel(next.state, false), Some(before));
    }

    #[test]
    fn cancelled_preedit_keeps_redo_and_context_reset_discards_its_baseline() {
        let before = snapshot("A", 1..1);
        let after = snapshot("AB", 2..2);
        let mut history = EditHistory::default();
        history.record(before.clone());
        let restored = history.travel(after.clone(), false).unwrap();
        history.begin_composition(restored.clone());
        history.finish_composition(&restored.text);
        assert_eq!(history.travel(restored.clone(), true), Some(after));
        history.begin_composition(restored);
        assert_eq!(history.committed_value("A你"), "A");
        history = EditHistory::default(); // set_context_value/reset
        assert_eq!(history.committed_value("other room"), "other room");
        assert!(history.travel(before, false).is_none());
    }

    #[test]
    fn concealed_geometry_maps_graphemes_instead_of_utf8_byte_counts() {
        let text = "äe\u{301}👩‍👩‍👧‍👧";
        let concealed = ConcealedText::new(text);
        assert_eq!(concealed.text.as_ref(), "•••");
        for (source, display) in [(0, 0), (2, 3), (5, 6), (text.len(), 9)] {
            assert_eq!(concealed.display_offset(source), display);
            assert_eq!(concealed.source_offset(display), source);
        }
        assert_eq!(concealed.display_offset(3), 3);
    }

    #[test]
    fn word_navigation_preserves_unicode_and_skips_punctuation() {
        let text = "Grüße, @Zoë! 👩‍👩‍👧‍👧 fin";
        assert_eq!(&text[..word_boundary(text, 0, true)], "Grüße");
        let end = word_boundary(text, 7, true);
        assert_eq!(&text[10..end], "Zoë");
        assert_eq!(word_boundary(text, end, false), 10);
        assert_eq!(word_boundary(text, text.len(), true), text.len());
    }
}
