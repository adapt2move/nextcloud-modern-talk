//! Read-only text interaction. One selection per desktop, never one entity per
//! virtualized message. Rendered elements supply current text and copy policy;
//! an unmounted, replaced or newly restricted row has no clipboard authority.
use std::ops::Range;

use gpui::{
    App, Context, CursorStyle, DispatchPhase, ElementId, Entity, FocusHandle, HighlightStyle,
    IntoElement, KeyBinding, MouseButton, MouseMoveEvent, MouseUpEvent, Pixels, Point, RenderOnce,
    SharedString, StyledText, TextLayout, Window, div, prelude::*,
};
use unicode_segmentation::UnicodeSegmentation;

use super::{input, theme::Theme};

const MAX_BYTES: usize = 4096;
const MAX_LINES: usize = 256;

#[derive(Default)]
struct SelectionState {
    target: Option<(ElementId, SharedString)>,
    anchor: usize,
    head: usize,
    dragging: bool,
}

impl SelectionState {
    fn matches(&self, id: &ElementId, text: &str) -> bool {
        self.target
            .as_ref()
            .is_some_and(|(key, value)| key == id && value.as_ref() == text)
    }

    fn range(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    fn begin(
        &mut self,
        id: ElementId,
        text: SharedString,
        index: usize,
        clicks: usize,
        shift: bool,
    ) {
        if text.len() > MAX_BYTES {
            return;
        }
        let index = boundary(&text, index);
        if !shift || !self.matches(&id, &text) {
            self.anchor = index;
        }
        self.head = index;
        if clicks >= 3 {
            self.anchor = 0;
            self.head = text.len();
        } else if clicks == 2 {
            let word = text
                .unicode_word_indices()
                .find(|(start, word)| *start <= index && index < *start + word.len())
                .map(|(start, word)| start..start + word.len())
                .unwrap_or(index..index);
            self.anchor = word.start;
            self.head = word.end;
        }
        self.target = Some((id, text));
        self.dragging = clicks == 1;
    }

    fn copied<'a>(&self, id: &ElementId, text: &'a str, permitted: bool) -> Option<&'a str> {
        (permitted && self.matches(id, text) && !self.range().is_empty())
            .then(|| text.get(self.range()))
            .flatten()
    }

    fn extend(&mut self, edge: Edge) {
        let Some((_, text)) = &self.target else {
            return;
        };
        self.head = match edge {
            Edge::Left => text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .take_while(|offset| *offset < self.head)
                .last()
                .unwrap_or(0),
            Edge::Right => text
                .grapheme_indices(true)
                .map(|(offset, _)| offset)
                .find(|offset| *offset > self.head)
                .unwrap_or(text.len()),
            Edge::Start => 0,
            Edge::End => text.len(),
        };
        self.dragging = false;
    }
}

#[derive(Clone, Copy)]
enum Edge {
    Left,
    Right,
    Start,
    End,
}

fn extend<A>(
    owner: Entity<Selection>,
    key: ElementId,
    text: SharedString,
    edge: Edge,
) -> impl Fn(&A, &mut Window, &mut App) {
    move |_, _, cx| {
        owner.update(cx, |owner, cx| {
            if owner.state.matches(&key, &text) {
                owner.state.extend(edge);
                cx.notify();
            }
        });
        cx.stop_propagation();
    }
}

fn boundary(text: &str, index: usize) -> usize {
    text.grapheme_indices(true)
        .map(|(index, _)| index)
        .chain(std::iter::once(text.len()))
        .take_while(|offset| *offset <= index)
        .last()
        .unwrap_or(0)
}

pub(super) struct Selection {
    state: SelectionState,
    focus: FocusHandle,
    live: bool,
}

impl Selection {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            state: SelectionState::default(),
            focus: cx.focus_handle().tab_stop(false),
            live: false,
        }
    }
    pub fn begin_frame(&mut self) {
        self.live = false;
    }
    pub fn finish_frame(
        &mut self,
        window: &mut Window,
        fallback: &FocusHandle,
        cx: &mut Context<Self>,
    ) {
        if !self.live {
            if self.focus.is_focused(window) {
                fallback.focus(window);
            }
            if self.state.target.is_some() {
                self.clear(cx);
            }
        }
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.state = SelectionState::default();
        cx.notify();
    }
}

struct Line {
    range: Range<usize>,
    code: bool,
    quote: bool,
}

struct Block {
    text: SharedString,
    lines: Vec<Line>,
}

impl Block {
    fn new(text: &str, rich: bool) -> Self {
        if !rich {
            return Self {
                text: text.to_owned().into(),
                lines: vec![Line {
                    range: 0..text.len(),
                    code: false,
                    quote: false,
                }],
            };
        }
        let mut display = String::with_capacity(text.len());
        let mut lines = Vec::new();
        let mut code = false;
        for line in text.split('\n').take(MAX_LINES) {
            if line.starts_with("```") {
                code = !code;
                continue;
            }
            if !lines.is_empty() {
                display.push('\n');
            }
            let quote = line.strip_prefix("> ");
            let start = display.len();
            display.push_str(quote.unwrap_or(line));
            lines.push(Line {
                range: start..display.len(),
                code,
                quote: quote.is_some(),
            });
        }
        Self {
            text: display.into(),
            lines,
        }
    }
}

#[derive(IntoElement)]
pub(super) struct SelectableText {
    id: ElementId,
    block: Block,
    selection: Entity<Selection>,
    theme: Theme,
    permitted: bool,
    rich: bool,
    scale: f32,
}

impl SelectableText {
    pub fn new(
        id: impl Into<ElementId>,
        text: &str,
        selection: Entity<Selection>,
        theme: Theme,
        permitted: bool,
    ) -> Self {
        Self {
            id: id.into(),
            block: Block::new(text, false),
            selection,
            theme,
            permitted: permitted && text.len() <= MAX_BYTES,
            rich: false,
            scale: 1.,
        }
    }
    pub fn rich(mut self, text: &str, scale: f32) -> Self {
        self.block = Block::new(text, true);
        self.rich = true;
        self.scale = scale;
        self
    }
}

fn mouse_index(
    layouts: &[(Range<usize>, TextLayout)],
    position: Point<Pixels>,
    text: &str,
) -> usize {
    let Some((range, layout)) = layouts
        .iter()
        .find(|(_, layout)| position.y <= layout.bounds().bottom())
        .or_else(|| layouts.last())
    else {
        return 0;
    };
    let local = layout
        .index_for_position(position)
        .unwrap_or_else(|index| index);
    boundary(text, range.start + local.min(range.len()))
}

impl RenderOnce for SelectableText {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let Self {
            id,
            block,
            selection,
            theme,
            permitted,
            rich,
            scale,
        } = self;
        let selected = permitted && selection.read(cx).state.matches(&id, &block.text);
        let focus = selection.read(cx).focus.clone();
        let range = selected.then(|| selection.read(cx).state.range());
        if selected {
            selection.update(cx, |owner, _| owner.live = true);
        }
        let mut body = div().min_w_0().when(rich, |d| {
            d.flex()
                .flex_col()
                .gap_1()
                .line_height(gpui::px(22. * scale))
        });
        let mut layouts = Vec::with_capacity(block.lines.len());
        for line in block.lines {
            let text = StyledText::new(block.text[line.range.clone()].to_owned()).with_highlights(
                range.as_ref().and_then(|range| {
                    let start = range.start.max(line.range.start);
                    let end = range.end.min(line.range.end);
                    (start < end).then_some((
                        start.saturating_sub(line.range.start)
                            ..end.saturating_sub(line.range.start),
                        HighlightStyle {
                            background_color: Some(theme.selected),
                            ..Default::default()
                        },
                    ))
                }),
            );
            layouts.push((line.range, text.layout().clone()));
            body = body.child(
                div()
                    .min_w_0()
                    .when(rich, |d| d.min_h(gpui::px(22. * scale)))
                    .when(line.code, |d| {
                        d.font_family("monospace").bg(theme.subtle).px_2()
                    })
                    .when(line.quote, |d| {
                        d.pl_3().border_l_2().border_color(theme.border)
                    })
                    .child(text),
            );
        }
        if !permitted {
            return body.into_any_element();
        }

        // GPUI requires global event listeners to be registered during paint,
        // after these text layouts have been prepainted. RenderOnce runs during
        // layout, so a noninteractive canvas owns registration for this frame.
        // Dropping the virtual row removes its listeners and layout handles.
        if selected && selection.read(cx).state.dragging {
            let owner = selection.clone();
            let key = id.clone();
            let text = block.text.clone();
            let layouts = layouts.clone();
            let release_owner = selection.clone();
            body = body.relative().child(
                gpui::canvas(
                    |_, _, _| (),
                    move |_, _, window, _| {
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble {
                                owner.update(cx, |owner, cx| {
                                    if owner.state.matches(&key, &text) && owner.state.dragging {
                                        if event.pressed_button != Some(MouseButton::Left) {
                                            owner.state.dragging = false;
                                        } else {
                                            owner.state.head =
                                                mouse_index(&layouts, event.position, &text);
                                        }
                                        cx.notify();
                                    }
                                });
                            }
                        });
                        window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                            if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                                release_owner.update(cx, |owner, _| owner.state.dragging = false);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            );
        }
        let owner = selection.clone();
        let key = id.clone();
        let text = block.text.clone();
        let copy_owner = selection.clone();
        let copy_key = id.clone();
        let copy_text = block.text.clone();
        let all_owner = selection.clone();
        let all_key = id.clone();
        let all_text = block.text;
        body.id(id)
            .cursor(CursorStyle::IBeam)
            .when(selected, |d| {
                d.track_focus(&focus).key_context("SelectableText")
            })
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                owner.update(cx, |owner, cx| {
                    owner.state.begin(
                        key.clone(),
                        text.clone(),
                        mouse_index(&layouts, event.position, &text),
                        event.click_count,
                        event.modifiers.shift,
                    );
                    owner.focus.focus(window);
                    cx.notify();
                });
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_action(move |_: &input::Copy, _, cx| {
                if let Some(text) = copy_owner
                    .read(cx)
                    .state
                    .copied(&copy_key, &copy_text, permitted)
                {
                    input::copy_text_selection(text, 0..text.len(), permitted, MAX_BYTES, cx);
                }
                cx.stop_propagation();
            })
            .on_action(extend::<input::SelectLeft>(
                selection.clone(),
                all_key.clone(),
                all_text.clone(),
                Edge::Left,
            ))
            .on_action(extend::<input::SelectRight>(
                selection.clone(),
                all_key.clone(),
                all_text.clone(),
                Edge::Right,
            ))
            .on_action(extend::<input::SelectDocumentStart>(
                selection.clone(),
                all_key.clone(),
                all_text.clone(),
                Edge::Start,
            ))
            .on_action(extend::<input::SelectDocumentEnd>(
                selection,
                all_key.clone(),
                all_text.clone(),
                Edge::End,
            ))
            .on_action(move |_: &input::SelectAll, _, cx| {
                all_owner.update(cx, |owner, cx| {
                    if owner.state.matches(&all_key, &all_text) {
                        owner.state.anchor = 0;
                        owner.state.head = all_text.len();
                        cx.notify();
                    }
                });
                cx.stop_propagation();
            })
            .into_any_element()
    }
}

pub(super) fn bind_keys(cx: &mut App) {
    let primary = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{primary}-c"), input::Copy, Some("SelectableText")),
        KeyBinding::new(
            &format!("{primary}-a"),
            input::SelectAll,
            Some("SelectableText"),
        ),
        KeyBinding::new("shift-left", input::SelectLeft, Some("SelectableText")),
        KeyBinding::new("shift-right", input::SelectRight, Some("SelectableText")),
        KeyBinding::new(
            &format!("{primary}-shift-home"),
            input::SelectDocumentStart,
            Some("SelectableText"),
        ),
        KeyBinding::new(
            &format!("{primary}-shift-end"),
            input::SelectDocumentEnd,
            Some("SelectableText"),
        ),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new(
            "cmd-shift-up",
            input::SelectDocumentStart,
            Some("SelectableText"),
        ),
        KeyBinding::new(
            "cmd-shift-down",
            input::SelectDocumentEnd,
            Some("SelectableText"),
        ),
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_requires_same_target_text_and_current_policy() {
        let mut state = SelectionState::default();
        state.begin("account-a-message-1".into(), "Hallo 🌍".into(), 0, 3, false);
        assert_eq!(
            state.copied(&"account-a-message-1".into(), "Hallo 🌍", true),
            Some("Hallo 🌍")
        );
        assert_eq!(
            state.copied(&"account-b-message-1".into(), "Hallo 🌍", true),
            None
        );
        assert_eq!(
            state.copied(&"account-a-message-1".into(), "Gelöscht", true),
            None
        );
        assert_eq!(
            state.copied(&"account-a-message-1".into(), "Hallo 🌍", false),
            None
        );
        state.begin("server-error".into(), "HTTP 403".into(), 0, 3, false);
        assert_eq!(
            state.copied(&"account-a-message-1".into(), "Hallo 🌍", true),
            None
        );
    }

    #[test]
    fn selection_preserves_graphemes_words_and_reversed_ranges() {
        let mut state = SelectionState::default();
        let text: SharedString = "Grüße 👨‍👩‍👧‍👦 e\u{301}".into();
        state.begin("text".into(), text.clone(), 3, 2, false);
        assert_eq!(state.copied(&"text".into(), &text, true), Some("Grüße"));
        state.begin("text".into(), text.clone(), text.len(), 1, false);
        state.begin("text".into(), text.clone(), 9, 1, true);
        assert_eq!(
            state.copied(&"text".into(), &text, true),
            Some("👨‍👩‍👧‍👦 e\u{301}")
        );
        state.extend(Edge::Right);
        assert_eq!(state.copied(&"text".into(), &text, true), Some(" e\u{301}"));
        state.extend(Edge::End);
        assert_eq!(state.copied(&"text".into(), &text, true), None);
        state.begin(
            "too-large".into(),
            "x".repeat(MAX_BYTES + 1).into(),
            0,
            3,
            false,
        );
        assert!(state.matches(&"text".into(), &text));
    }

    #[test]
    fn rich_copy_uses_visible_text_and_keeps_line_breaks() {
        let block = Block::new("one\n> Grüße\n```rust\ncode\n```\n\nlast", true);
        assert_eq!(block.text.as_ref(), "one\nGrüße\ncode\n\nlast");
        assert_eq!(&block.text[block.lines[1].range.clone()], "Grüße");
        assert!(block.lines[1].quote);
        assert!(block.lines[2].code);
        let bounded = Block::new(&"x\n".repeat(300), true);
        assert_eq!(bounded.lines.len(), MAX_LINES);
    }
}
