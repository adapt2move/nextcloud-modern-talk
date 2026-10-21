// Adapted from GPUI 0.2.2 examples/input.rs, Zed Industries, Apache-2.0.
// See assets/GPUI-LICENSE-APACHE and docs/design/native-ui/dependencies.md.
// Changes: finite UTF-8 input budget, local error, appearance/focus, corrected IME
// replacement offsets, no key logging, bounded clipboard admission.
use std::ops::Range;
use std::time::Duration;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, SharedString, Style,
    Subscription, Task, TextRun, Timer, UTF16Selection, UnderlineStyle, Window, WrappedLine,
    actions, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Up,
        Down,
        SelectUp,
        SelectDown,
        NewLine,
        DocumentStart,
        DocumentEnd,
        SelectHome,
        SelectEnd,
        SelectDocumentStart,
        SelectDocumentEnd,
        Undo,
        Redo,
    ]
);

// Two finite histories per user-entry widget: <= 16 × its byte limit each.
// No history crosses set_value/reset (account/conversation projection changes).
#[derive(Clone)]
struct EditSnapshot {
    text: SharedString,
    selection: Range<usize>,
    reversed: bool,
}
#[derive(Default)]
struct EditHistory {
    undo: Vec<EditSnapshot>,
    redo: Vec<EditSnapshot>,
}
impl EditHistory {
    fn push(list: &mut Vec<EditSnapshot>, state: EditSnapshot) {
        if list.len() == 16 {
            list.remove(0);
        }
        list.push(state);
    }
    fn record(&mut self, state: EditSnapshot) {
        Self::push(&mut self.undo, state);
        self.redo.clear();
    }
    fn travel(&mut self, current: EditSnapshot, redo: bool) -> Option<EditSnapshot> {
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

const CARET_INTERVAL: Duration = Duration::from_millis(500);
const CARET_TICKS: u8 = 20;

fn caret_height(ascent: Pixels, descent: Pixels, font_size: Pixels, line_height: Pixels) -> Pixels {
    // Font metrics can expose the signed OpenType descender. Its distance is
    // part of the caret, not something to subtract from the ascender.
    (ascent + px(f32::from(descent).abs()))
        .max(font_size)
        .clamp(px(1.), line_height.max(px(1.)))
}

fn horizontal_scroll(
    previous: Pixels,
    cursor: Pixels,
    text_width: Pixels,
    viewport_width: Pixels,
) -> Pixels {
    let visible = (viewport_width - px(4.)).max(px(1.));
    // A wider field or shorter replacement must reveal text that now fits.
    // Retaining the previous offset can otherwise hide the start indefinitely.
    previous
        .max(cursor - visible)
        .min(cursor)
        .clamp(px(0.), (text_width - visible).max(px(0.)))
}

/// One finite blink sequence per focused input. A new edit invalidates the old
/// generation; a hidden view cannot keep blinking without presenting each tick.
struct CaretBlink {
    generation: u64,
    remaining: u8,
    visible: bool,
    restart: bool,
    painted: Option<(u64, u8)>,
}
impl Default for CaretBlink {
    fn default() -> Self {
        Self {
            generation: 0,
            remaining: 0,
            visible: true,
            restart: true,
            painted: None,
        }
    }
}
impl CaretBlink {
    fn reset(&mut self, restart: bool) {
        self.generation = self.generation.wrapping_add(1);
        self.remaining = 0;
        self.visible = true;
        self.restart = restart;
        self.painted = None;
    }
    fn begin(&mut self) -> u64 {
        self.remaining = CARET_TICKS;
        self.restart = false;
        self.generation
    }
    fn presented(&mut self) {
        self.painted = Some((self.generation, self.remaining));
    }
    fn tick(&mut self, generation: u64) -> bool {
        if generation != self.generation {
            return false;
        }
        if self.remaining == 0 || self.painted != Some((generation, self.remaining)) {
            self.reset(false);
            return false;
        }
        self.remaining -= 1;
        self.visible = self.remaining == 0 || !self.visible;
        self.remaining > 0
    }
}

pub struct TextInput {
    /// Ephemeral password entry. Masking preserves byte offsets used by the input contract.
    pub concealed: bool,
    pub interactive: bool,
    history: EditHistory,
    pub borderless: bool,
    pub multiline: bool,
    focus_handle: FocusHandle,
    pub content: SharedString,
    pub limit: usize,
    pub rejected: bool,
    pub rejection: Option<&'static str>,
    pub theme: super::theme::Theme,
    pub text_scale: f32,
    /// User-entry clipboard only. Composer requires explicit projected privacy permission.
    pub clipboard_allowed: bool,
    scroll_x: Pixels,
    scroll_y: Pixels,
    visible_rows: usize,
    pub max_visible_rows: usize,
    desired_x: Option<Pixels>,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<TextLayout>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    caret: CaretBlink,
    caret_task: Option<Task<()>>,
    caret_subscriptions: Vec<Subscription>,
    caret_reduced_motion: bool,
}

impl TextInput {
    fn restart_caret(&mut self) {
        self.caret_task.take();
        self.caret.reset(true);
    }

    fn stop_caret(&mut self) {
        self.caret_task.take();
        self.caret.reset(false);
    }

    /// Called after the parent's final interactivity projection, including for
    /// inputs hidden by a modal. No background view keeps an active blink task.
    pub fn configure_caret(&mut self, reduced_motion: bool) {
        if self.caret_reduced_motion != reduced_motion {
            self.caret_reduced_motion = reduced_motion;
            self.restart_caret();
        }
        if !self.interactive
            || reduced_motion
            || !self.selected_range.is_empty()
            || self.composing()
        {
            self.stop_caret();
        }
    }

    fn caret_can_blink(&self, window: &Window) -> bool {
        self.interactive
            && !self.caret_reduced_motion
            && self.selected_range.is_empty()
            && !self.composing()
            && window.is_window_active()
            && self.focus_handle.is_focused(window)
    }

    /// Sole foreground timer seam for this view: one retained weak task, at
    /// most 20 half-second ticks. No I/O, detached task or animation-frame loop.
    fn schedule_caret_blink(&mut self, window: &Window, cx: &mut Context<Self>) {
        let generation = self.caret.begin();
        self.caret_task = Some(cx.spawn_in(window, async move |input, cx| {
            for _ in 0..CARET_TICKS {
                Timer::after(CARET_INTERVAL).await;
                let keep_blinking = input.update_in(cx, |input, window, cx| {
                    if input.caret.generation != generation {
                        return false;
                    }
                    if !input.caret_can_blink(window) {
                        input.caret.reset(false);
                        return false;
                    }
                    let keep_blinking = input.caret.tick(generation);
                    cx.notify();
                    keep_blinking
                });
                if !matches!(keep_blinking, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn sync_caret(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.caret_subscriptions.is_empty() {
            self.caret_subscriptions = vec![
                cx.on_focus(&self.focus_handle, window, |input, _, cx| {
                    input.restart_caret();
                    cx.notify();
                }),
                cx.on_blur(&self.focus_handle, window, |input, _, cx| {
                    input.stop_caret();
                    cx.notify();
                }),
                cx.observe_window_activation(window, |input, window, cx| {
                    if window.is_window_active() && input.focus_handle.is_focused(window) {
                        input.restart_caret();
                    } else {
                        input.stop_caret();
                    }
                    cx.notify();
                }),
            ];
        }
        if !self.caret_can_blink(window) {
            self.stop_caret();
        } else if self.caret.restart {
            self.schedule_caret_blink(window, cx);
        }
    }

    pub(super) fn mention_query(&self) -> Option<&str> {
        if self.composing() || !self.selected_range.is_empty() {
            return None;
        }
        let range = mention_range(&self.content, self.cursor_offset())?;
        Some(&self.content[range.start + 1..range.end])
    }

    pub(super) fn insert_mention(&mut self, identifier: &str, cx: &mut Context<Self>) {
        if self.composing() || !self.selected_range.is_empty() {
            return;
        }
        let Some(range) = mention_range(&self.content, self.cursor_offset()) else {
            return;
        };
        let replacement = format!("@{identifier} ");
        if !replacement_fits(
            &self.content,
            &range,
            &replacement,
            self.limit,
            self.multiline,
        ) {
            self.rejected = true;
            cx.notify();
            return;
        }
        self.history.record(self.edit_snapshot());
        self.content = format!(
            "{}{}{}",
            &self.content[..range.start],
            replacement,
            &self.content[range.end..]
        )
        .into();
        let cursor = range.start + replacement.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.rejected = false;
        self.invalidate_geometry();
        cx.notify();
    }

    fn edit_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            text: self.content.clone(),
            selection: self.selected_range.clone(),
            reversed: self.selection_reversed,
        }
    }
    fn history_travel(&mut self, redo: bool, cx: &mut Context<Self>) {
        if !self.interactive || self.composing() {
            return;
        }
        if let Some(previous) = self.history.travel(self.edit_snapshot(), redo) {
            self.content = previous.text;
            self.selected_range = previous.selection;
            self.selection_reversed = previous.reversed;
            self.invalidate_geometry();
            self.desired_x = None;
            self.rejected = false;
            self.rejection = None;
            cx.notify();
        }
    }
    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.history_travel(false, cx);
    }
    fn redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.history_travel(true, cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx)
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx)
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let start = if self.multiline {
            self.content[..self.cursor_offset()]
                .rfind('\n')
                .map_or(0, |i| i + 1)
        } else {
            0
        };
        self.move_to(start, cx);
    }
    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let end = if self.multiline {
            self.content[cursor..]
                .find('\n')
                .map_or(self.content.len(), |i| cursor + i)
        } else {
            self.content.len()
        };
        self.move_to(end, cx);
    }
    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let start = if self.multiline {
            self.content[..self.cursor_offset()]
                .rfind('\n')
                .map_or(0, |i| i + 1)
        } else {
            0
        };
        self.select_to(start, cx);
    }
    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let end = if self.multiline {
            self.content[cursor..]
                .find('\n')
                .map_or(self.content.len(), |i| cursor + i)
        } else {
            self.content.len()
        };
        self.select_to(end, cx);
    }
    fn select_document_start(
        &mut self,
        _: &SelectDocumentStart,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(0, cx);
    }
    fn select_document_end(
        &mut self,
        _: &SelectDocumentEnd,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_to(self.content.len(), cx);
    }
    fn document_start(&mut self, _: &DocumentStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
    }
    fn document_end(&mut self, _: &DocumentEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), cx);
    }
    fn vertical(&mut self, down: bool, select: bool, cx: &mut Context<Self>) {
        if !self.multiline || self.composing() {
            return;
        }
        let Some(layout) = self.last_layout.as_ref() else {
            return;
        };
        let position = layout.position(self.cursor_offset());
        let x = self.desired_x.unwrap_or(position.x);
        let index = layout
            .index(point(
                x,
                position.y + layout.line_height * if down { 1.5 } else { -0.5 },
            ))
            .min(self.content.len());
        if select {
            self.select_to(index, cx)
        } else {
            self.move_to(index, cx)
        };
        self.desired_x = Some(x);
    }
    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, false, cx);
    }
    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, false, cx);
    }
    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(false, true, cx);
    }
    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(true, true, cx);
    }
    fn new_line(&mut self, _: &NewLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.multiline && !self.composing() {
            self.replace_text_in_range(None, "\n", window, cx);
        }
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.interactive {
            return;
        }
        self.focus_handle.focus(_window);
        self.is_selecting = true;
        if event.click_count == 2 {
            let cursor = self.index_for_mouse_position(event.position);
            let range = self
                .content
                .unicode_word_indices()
                .find(|(start, word)| *start <= cursor && cursor < *start + word.len())
                .map(|(start, word)| start..start + word.len())
                .unwrap_or(cursor..cursor);
            self.move_to(range.start, cx);
            self.select_to(range.end, cx);
            return;
        }
        if event.click_count >= 3 {
            self.move_to(0, cx);
            self.select_to(self.content.len(), cx);
            return;
        }

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        if self.interactive && !self.composing() && cfg!(target_os = "macos") {
            window.show_character_palette();
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if !self.clipboard_allowed {
            self.rejected = true;
            self.rejection = Some("Einfügen ist für diesen Entwurf nicht freigegeben.");
            cx.notify();
            return;
        }
        // GPUI materializes the platform clipboard before this finite admission check.
        // This transient native allocation cannot be bounded through the 0.2.2 API.
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            if text.len() > self.limit {
                self.rejected = true;
                self.rejection =
                    Some("Der eingefügte Text ist zu lang. Dein Entwurf bleibt erhalten.");
                cx.notify();
                return;
            }
            let text = if self.multiline {
                text.replace("\r\n", "\n").replace('\r', "\n")
            } else {
                text.replace(['\n', '\r'], " ")
            };
            self.replace_text_in_range(None, &text, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.clipboard_allowed {
            self.rejected = true;
            self.rejection = Some("Kopieren ist für diesen Entwurf nicht freigegeben.");
            cx.notify();
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.clipboard_allowed {
            self.rejected = true;
            self.rejection = Some("Ausschneiden ist für diesen Entwurf nicht freigegeben.");
            cx.notify();
            return;
        }
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = current_boundary(&self.content, offset);
        self.desired_x = None;
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.restart_caret();
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref())
        else {
            return self.cursor_offset();
        };
        line.index(point(
            position.x - bounds.left() + self.scroll_x,
            position.y - bounds.top() + self.scroll_y,
        ))
        .min(self.content.len())
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        let offset = current_boundary(&self.content, offset);
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.restart_caret();
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        from_utf16(&self.content, offset)
    }
    // Hit-test positions belong to the exact painted text. An edit or undo
    // retires them until the next paint; stale byte positions are never sliced.
    fn invalidate_geometry(&mut self) {
        self.last_layout = None;
        self.last_bounds = None;
        self.restart_caret();
    }
    fn offset_to_utf16(&self, offset: usize) -> usize {
        to_utf16(&self.content, offset)
    }
    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }
    fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        let start = self.offset_from_utf16(range.start);
        let end = self.offset_from_utf16(range.end);
        start.min(end)..start.max(end)
    }
    fn previous_boundary(&self, offset: usize) -> usize {
        previous_grapheme(&self.content, offset)
    }
    fn next_boundary(&self, offset: usize) -> usize {
        next_grapheme(&self.content, offset)
    }

    pub fn reset(&mut self) {
        self.restart_caret();
        self.history = EditHistory::default();
        self.rejected = false;
        self.rejection = None;
        self.scroll_x = px(0.);
        self.scroll_y = px(0.);
        self.visible_rows = 1;
        self.desired_x = None;
        self.content = "".into();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        self.last_bounds = None;
        self.is_selecting = false;
    }
}

fn mention_range(text: &str, cursor: usize) -> Option<Range<usize>> {
    let before = text.get(..cursor)?;
    let start = before.rfind('@')?;
    let query = before.get(start + 1..)?;
    if query.len() > 256
        || query.chars().any(char::is_whitespace)
        || before[..start]
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace() && !matches!(c, '(' | '[' | '{'))
    {
        return None;
    }
    Some(start..cursor)
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        self.restart_caret();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.interactive {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if !replacement_fits(&self.content, &range, new_text, self.limit, self.multiline) {
            self.rejected = true;
            cx.notify();
            return;
        }
        if self.marked_range.is_none() {
            self.history.record(self.edit_snapshot());
        }
        self.rejected = false;
        self.rejection = None;
        self.desired_x = None;
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.invalidate_geometry();
        self.marked_range.take();
        self.selection_reversed = false;
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.interactive {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        if !replacement_fits(&self.content, &range, new_text, self.limit, self.multiline) {
            self.rejected = true;
            cx.notify();
            return;
        }
        if self.marked_range.is_none() {
            self.history.record(self.edit_snapshot());
        }
        self.rejected = false;
        self.rejection = None;
        self.desired_x = None;
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.invalidate_geometry();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|r| {
                let start = from_utf16(new_text, r.start);
                let end = from_utf16(new_text, r.end);
                range.start + start.min(end)..range.start + start.max(end)
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.last_layout.as_ref()?;
        let range = self.range_from_utf16(&range_utf16);
        let start = layout.position(range.start);
        let end = layout.position(range.end);
        // IME candidate placement is on the first affected visual row; a range
        // crossing lines must not produce a reversed native rectangle.
        Some(Bounds::new(
            point(
                bounds.left() + start.x - self.scroll_x,
                bounds.top() + start.y - self.scroll_y,
            ),
            size(
                if start.y == end.y {
                    (end.x - start.x).max(px(2.))
                } else {
                    px(2.)
                },
                layout.line_height,
            ),
        ))
    }
    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        let layout = self.last_layout.as_ref()?;
        let index = layout
            .index(point(
                position.x - bounds.left() + self.scroll_x,
                position.y - bounds.top() + self.scroll_y,
            ))
            .min(self.content.len());
        Some(self.offset_to_utf16(index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

// At most limit+1 paragraphs and limit+1 visual rows: every extra break
// consumes admitted UTF-8 bytes. Layout is replaced, never accumulated.
struct TextLayout {
    lines: Vec<WrappedLine>,
    starts: Vec<usize>,
    tops: Vec<Pixels>,
    line_height: Pixels,
    height: Pixels,
}
impl TextLayout {
    fn new(lines: Vec<WrappedLine>, line_height: Pixels) -> Self {
        let mut starts = Vec::with_capacity(lines.len());
        let mut tops = Vec::with_capacity(lines.len());
        let mut start = 0;
        let mut height = px(0.);
        for line in &lines {
            starts.push(start);
            tops.push(height);
            start += line.len() + 1;
            height += line.size(line_height).height;
        }
        Self {
            lines,
            starts,
            tops,
            line_height,
            height,
        }
    }
    fn position(&self, index: usize) -> Point<Pixels> {
        for (i, line) in self.lines.iter().enumerate() {
            if index <= self.starts[i] + line.len() {
                let p = line
                    .position_for_index(index.saturating_sub(self.starts[i]), self.line_height)
                    .unwrap_or_default();
                return point(p.x, self.tops[i] + p.y);
            }
        }
        point(px(0.), (self.height - self.line_height).max(px(0.)))
    }
    fn index(&self, position: Point<Pixels>) -> usize {
        if position.y < px(0.) {
            return 0;
        }
        for (i, line) in self.lines.iter().enumerate() {
            if position.y < self.tops[i] + line.size(self.line_height).height {
                let result = line.closest_index_for_position(
                    point(position.x, position.y - self.tops[i]),
                    self.line_height,
                );
                return self.starts[i] + result.unwrap_or_else(|near| near);
            }
        }
        self.lines
            .last()
            .map_or(0, |l| self.starts[self.lines.len() - 1] + l.len())
    }
    fn selections(&self, range: Range<usize>, width: Pixels) -> Vec<Bounds<Pixels>> {
        let mut rectangles = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            let mut boundaries = vec![0];
            boundaries.extend(
                line.wrap_boundaries()
                    .iter()
                    .map(|b| line.runs()[b.run_ix].glyphs[b.glyph_ix].index),
            );
            boundaries.push(line.len());
            for (row, segment) in boundaries.windows(2).enumerate() {
                let a = range.start.saturating_sub(self.starts[i]).max(segment[0]);
                let b = range.end.saturating_sub(self.starts[i]).min(segment[1]);
                let selected_newline = range.start <= self.starts[i] + line.len()
                    && range.end > self.starts[i] + line.len()
                    && row + 2 == boundaries.len();
                if a >= b && !selected_newline {
                    continue;
                }
                let x0 = line.unwrapped_layout.x_for_index(segment[0]);
                let start = line.unwrapped_layout.x_for_index(a.min(segment[1])) - x0;
                let end = if selected_newline {
                    width
                } else {
                    line.unwrapped_layout.x_for_index(b) - x0
                };
                rectangles.push(Bounds::new(
                    point(start, self.tops[i] + self.line_height * row),
                    size((end - start).max(px(2.)), self.line_height),
                ));
            }
        }
        rectangles
    }
}
struct PrepaintState {
    layout: Option<TextLayout>,
    cursor: Option<PaintQuad>,
    selection: Vec<PaintQuad>,
    scroll_x: Pixels,
    scroll_y: Pixels,
    visible_rows: usize,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = (window.line_height() * self.input.read(cx).visible_rows).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let style = window.text_style();
        let (text, color) = if input.content.is_empty() {
            (input.placeholder.clone(), input.theme.placeholder)
        } else if input.concealed {
            ("*".repeat(input.content.len()).into(), style.color)
        } else {
            (input.content.clone(), style.color)
        };
        let base = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked) = input.marked_range.as_ref() {
            vec![
                TextRun {
                    len: marked.start,
                    ..base.clone()
                },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle {
                        color: Some(color),
                        thickness: px(1.),
                        wavy: false,
                    }),
                    ..base.clone()
                },
                TextRun {
                    len: text.len() - marked.end,
                    ..base
                },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect()
        } else {
            vec![base]
        };
        let lines = window
            .text_system()
            .shape_text(
                text,
                style.font_size.to_pixels(window.rem_size()),
                &runs,
                if input.multiline {
                    Some(bounds.size.width.max(px(1.)))
                } else {
                    None
                },
                None,
            )
            .unwrap_or_default()
            .into_iter()
            .collect();
        let layout = TextLayout::new(lines, window.line_height());
        let cursor = layout.position(input.cursor_offset());
        let scroll_x = if input.multiline {
            px(0.)
        } else {
            horizontal_scroll(
                input.scroll_x,
                cursor.x,
                layout
                    .lines
                    .first()
                    .map_or(px(0.), |line| line.size(layout.line_height).width),
                bounds.size.width,
            )
        };
        let rows = if input.multiline {
            (layout.height / layout.line_height).ceil() as usize
        } else {
            1
        };
        let visible_rows = rows.clamp(1, self.input.read(cx).max_visible_rows);
        let viewport_height = layout.line_height * visible_rows;
        let scroll_y = if cursor.y < input.scroll_y {
            cursor.y
        } else if cursor.y + layout.line_height > input.scroll_y + viewport_height {
            cursor.y + layout.line_height - viewport_height
        } else {
            input.scroll_y
        }
        .clamp(px(0.), (layout.height - viewport_height).max(px(0.)));
        let selection = layout
            .selections(input.selected_range.clone(), bounds.size.width)
            .into_iter()
            .map(|r| {
                fill(
                    Bounds::new(bounds.origin + r.origin - point(scroll_x, scroll_y), r.size),
                    input.theme.selected,
                )
            })
            .collect();
        let caret = input.selected_range.is_empty().then(|| {
            let font_size = style.font_size.to_pixels(window.rem_size());
            let font = window.text_system().resolve_font(&style.font());
            let height = caret_height(
                window.text_system().ascent(font, font_size),
                window.text_system().descent(font, font_size),
                font_size,
                layout.line_height,
            );
            let origin = bounds.origin + cursor - point(scroll_x, scroll_y)
                + point(px(0.), (layout.line_height - height) / 2.);
            let scale = window.scale_factor();
            fill(
                Bounds::new(
                    point((origin.x * scale).round() / scale, origin.y),
                    size(px(1.), height),
                ),
                input.theme.text,
            )
        });
        PrepaintState {
            layout: Some(layout),
            cursor: caret,
            selection,
            scroll_x,
            scroll_y,
            visible_rows,
        }
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepaint: &mut PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.input.read(cx).focus_handle.clone();
        if self.input.read(cx).interactive {
            window.handle_input(
                &focus,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }
        let layout = prepaint.layout.take().unwrap();
        for selection in prepaint.selection.drain(..) {
            window.paint_quad(selection);
        }
        for (i, line) in layout.lines.iter().enumerate() {
            let origin = point(
                bounds.left() - prepaint.scroll_x,
                bounds.top() + layout.tops[i] - prepaint.scroll_y,
            );
            // GPUI applies the containing viewport mask to every native glyph.
            let _ = line.paint(
                origin,
                layout.line_height,
                gpui::TextAlign::Left,
                Some(bounds),
                window,
                cx,
            );
        }
        if self.input.read(cx).interactive
            && window.is_window_active()
            && focus.is_focused(window)
            && self.input.read(cx).caret.visible
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }
        let changed = self.input.read(cx).visible_rows != prepaint.visible_rows;
        self.input.update(cx, |input, _| {
            input.caret.presented();
            input.scroll_x = prepaint.scroll_x;
            input.scroll_y = prepaint.scroll_y;
            input.visible_rows = prepaint.visible_rows;
            input.last_layout = Some(layout);
            input.last_bounds = Some(bounds);
        });
        if changed {
            let input = self.input.downgrade();
            window.on_next_frame(move |_, cx| {
                let _ = input.update(cx, |_, cx| cx.notify());
            });
        }
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_caret(window, cx);
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx).tab_stop(self.interactive))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::new_line))
            .on_action(cx.listener(Self::document_start))
            .on_action(cx.listener(Self::document_end))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_document_start))
            .on_action(cx.listener(Self::select_document_end))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .rounded_md()
            .border_1()
            .border_color(if self.borderless {
                gpui::transparent_black()
            } else if self.rejected {
                self.theme.danger
            } else {
                self.theme.input_border
            })
            .when(!self.borderless, |d| {
                d.child(super::controls::focus_ring(
                    self.focus_handle.clone(),
                    self.theme,
                    6.,
                ))
            })
            .bg(self.theme.surface)
            .text_color(self.theme.text)
            .line_height(px(24. * self.text_scale))
            .text_size(px(14. * self.text_scale))
            .child(
                div()
                    .h(px(if self.borderless {
                        (24. * self.visible_rows as f32 + 8.) * self.text_scale
                    } else {
                        46. * self.text_scale
                    }))
                    .w_full()
                    .px(px(if self.borderless { 2. } else { 12. }))
                    .py(px(if self.borderless {
                        4. * self.text_scale
                    } else {
                        11. * self.text_scale
                    }))
                    .overflow_hidden()
                    .child(TextElement { input: cx.entity() }),
            )
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn geometry(&self) -> Option<Bounds<Pixels>> {
        self.last_bounds
    }
    pub fn new(
        placeholder: &str,
        limit: usize,
        theme: super::theme::Theme,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            concealed: false,
            focus_handle: cx.focus_handle().tab_stop(true),
            interactive: true,
            history: EditHistory::default(),
            borderless: true,
            multiline: false,
            content: "".into(),
            placeholder: placeholder.to_owned().into(),
            limit,
            rejected: false,
            rejection: None,
            theme,
            text_scale: 1.,
            clipboard_allowed: false,
            scroll_x: px(0.),
            scroll_y: px(0.),
            visible_rows: 1,
            max_visible_rows: 5,
            desired_x: None,
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            caret: CaretBlink::default(),
            caret_task: None,
            caret_subscriptions: Vec::new(),
            caret_reduced_motion: false,
        }
    }
    pub fn set_context_value(&mut self, value: &str) {
        self.reset();
        self.set_value(value);
    }
    pub fn set_value(&mut self, value: &str) {
        if value == self.content.as_ref() {
            return;
        }
        if value.len() <= self.limit {
            self.reset();
            self.content = value.to_owned().into();
            self.selected_range = value.len()..value.len();
        }
    }
    pub fn set_placeholder(&mut self, value: &str) {
        self.placeholder = value.to_owned().into();
    }
    pub fn composing(&self) -> bool {
        self.marked_range.is_some()
    }
}
pub fn bind_keys(cx: &mut App) {
    let context = Some("TextInput");
    use talk_core::preferences::shortcuts::{ActionId, Modifiers, SHORTCUTS};
    cx.bind_keys(SHORTCUTS.iter().filter_map(|shortcut| {
        let primary = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };
        let prefix = match shortcut.modifiers {
            Modifiers::None => String::new(),
            Modifiers::Shift => "shift-".to_owned(),
            Modifiers::Primary => format!("{primary}-"),
            Modifiers::PrimaryShift => format!("{primary}-shift-"),
        };
        let binding = format!("{prefix}{}", shortcut.key);
        match shortcut.action {
            ActionId::NewLine => Some(KeyBinding::new(&binding, NewLine, context)),
            ActionId::Undo => Some(KeyBinding::new(&binding, Undo, context)),
            ActionId::Redo => Some(KeyBinding::new(&binding, Redo, context)),
            _ => None,
        }
    }));
    cx.bind_keys([
        KeyBinding::new("up", Up, context),
        KeyBinding::new("down", Down, context),
        KeyBinding::new("shift-up", SelectUp, context),
        KeyBinding::new("shift-down", SelectDown, context),
        KeyBinding::new("shift-home", SelectHome, context),
        KeyBinding::new("shift-end", SelectEnd, context),
        KeyBinding::new("ctrl-shift-home", SelectDocumentStart, context),
        KeyBinding::new("ctrl-shift-end", SelectDocumentEnd, context),
        KeyBinding::new("cmd-left", Home, context),
        KeyBinding::new("cmd-right", End, context),
        KeyBinding::new("cmd-shift-left", SelectHome, context),
        KeyBinding::new("cmd-shift-right", SelectEnd, context),
        KeyBinding::new("cmd-up", DocumentStart, context),
        KeyBinding::new("cmd-down", DocumentEnd, context),
        KeyBinding::new("cmd-shift-up", SelectDocumentStart, context),
        KeyBinding::new("cmd-shift-down", SelectDocumentEnd, context),
        KeyBinding::new("ctrl-home", DocumentStart, context),
        KeyBinding::new("ctrl-end", DocumentEnd, context),
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", SelectLeft, context),
        KeyBinding::new("shift-right", SelectRight, context),
        KeyBinding::new("cmd-a", SelectAll, context),
        KeyBinding::new("ctrl-a", SelectAll, context),
        KeyBinding::new("cmd-c", Copy, context),
        KeyBinding::new("ctrl-c", Copy, context),
        KeyBinding::new("cmd-v", Paste, context),
        KeyBinding::new("ctrl-v", Paste, context),
        KeyBinding::new("cmd-x", Cut, context),
        KeyBinding::new("ctrl-x", Cut, context),
        KeyBinding::new("home", Home, context),
        KeyBinding::new("end", End, context),
    ]);
    #[cfg(target_os = "macos")]
    cx.bind_keys([KeyBinding::new(
        "ctrl-cmd-space",
        ShowCharacterPalette,
        context,
    )]);
}

fn from_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        if units >= offset {
            return i;
        }
        units += c.len_utf16();
    }
    text.len()
}
fn to_utf16(text: &str, offset: usize) -> usize {
    text.char_indices()
        .take_while(|(i, _)| *i < offset)
        .map(|(_, c)| c.len_utf16())
        .sum()
}
fn previous_grapheme(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .rev()
        .find_map(|(i, _)| (i < offset).then_some(i))
        .unwrap_or(0)
}
fn next_grapheme(text: &str, offset: usize) -> usize {
    text.grapheme_indices(true)
        .find_map(|(i, _)| (i > offset).then_some(i))
        .unwrap_or(text.len())
}
fn current_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}
fn replacement_fits(
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
#[cfg(test)]
mod tests {
    use super::{
        CARET_TICKS, CaretBlink, EditHistory, EditSnapshot, caret_height, current_boundary,
        from_utf16, horizontal_scroll, mention_range, next_grapheme, previous_grapheme,
        replacement_fits, to_utf16,
    };
    #[test]
    fn single_line_scroll_recovers_after_resize_or_replacement() {
        use gpui::px;
        let clipped = horizontal_scroll(px(0.), px(120.), px(120.), px(20.));
        assert_eq!(clipped, px(104.));
        assert_eq!(
            horizontal_scroll(clipped, px(120.), px(120.), px(354.)),
            px(0.)
        );
        assert_eq!(
            horizontal_scroll(px(200.), px(8.), px(8.), px(100.)),
            px(0.)
        );
        assert_eq!(
            horizontal_scroll(px(0.), px(180.), px(300.), px(100.)),
            px(84.)
        );
        assert_eq!(
            horizontal_scroll(px(84.), px(20.), px(300.), px(100.)),
            px(20.)
        );
    }
    #[test]
    fn caret_covers_signed_descenders_at_each_text_scale() {
        for scale in [1., 1.5, 2.] {
            let px = |value| gpui::px(value * scale);
            let height = caret_height(px(14.924), px(-4.2), px(14.), px(24.));
            assert!((f32::from(height) - 19.124 * scale).abs() < 0.001);
            assert_eq!(height, caret_height(px(14.924), px(4.2), px(14.), px(24.)));
            assert_eq!(caret_height(px(100.), px(-30.), px(14.), px(24.)), px(24.));
        }
    }
    #[test]
    fn caret_blink_settles_visible_and_stale_ticks_cannot_hide_a_new_edit() {
        let mut caret = CaretBlink::default();
        let old = caret.begin();
        caret.presented();
        assert!(caret.tick(old));
        assert!(!caret.visible);
        caret.reset(true);
        assert!(caret.visible);
        assert!(!caret.tick(old));
        assert!(caret.visible);
        let current = caret.begin();
        for tick in 0..CARET_TICKS {
            caret.presented();
            assert_eq!(caret.tick(current), tick + 1 < CARET_TICKS);
        }
        assert!(caret.visible);
        assert_eq!(caret.remaining, 0);
        assert!(!caret.restart);
    }

    #[test]
    fn caret_blink_stops_when_hidden_or_cancelled() {
        let mut caret = CaretBlink::default();
        let generation = caret.begin();
        caret.presented();
        assert!(caret.tick(generation));
        // An unmounted input did not paint the new tick.
        assert!(!caret.tick(generation));
        assert!(caret.visible);
        assert_eq!(caret.remaining, 0);
        caret.reset(true);
        let generation = caret.begin();
        caret.presented();
        caret.reset(false); // Blur, inert, selection, IME or reduced motion.
        assert!(!caret.tick(generation));
        assert!(caret.visible);
        assert!(!caret.restart);
    }

    #[test]
    fn typed_mentions_follow_the_caret_and_do_not_match_email_or_old_tokens() {
        assert_eq!(mention_range("@", 1), Some(0..1));
        assert_eq!(mention_range("Hallo @Zoë und Text", 11), Some(6..11));
        assert_eq!(mention_range("(@Mia)", 5), Some(1..5));
        assert_eq!(mention_range("mail@example.test", 17), None);
        assert_eq!(mention_range("@Mia fertig", 11), None);
        assert_eq!(mention_range("Hallo @Mia", 2), None);
        assert_eq!(mention_range("Grüße @Zoë", 3), None);
        let long = format!("@{}", "x".repeat(257));
        assert_eq!(mention_range(&long, long.len()), None);
    }
    #[test]
    fn editing_history_restores_text_and_selection_and_bounds_old_edits() {
        let mut history = EditHistory::default();
        let old = EditSnapshot {
            text: "A🧭\nB".into(),
            selection: 1..5,
            reversed: true,
        };
        history.record(old.clone());
        let current = EditSnapshot {
            text: "AX\nB".into(),
            selection: 2..2,
            reversed: false,
        };
        let restored = history.travel(current.clone(), false).unwrap();
        assert_eq!(restored.text, old.text);
        assert_eq!(restored.selection, 1..5);
        assert!(restored.reversed);
        assert_eq!(history.travel(restored, true).unwrap().text, current.text);
        for n in 0..40 {
            history.record(EditSnapshot {
                text: n.to_string().into(),
                selection: 0..0,
                reversed: false,
            });
        }
        let mut count = 0;
        while history.travel(current.clone(), false).is_some() {
            count += 1;
        }
        assert_eq!(count, 16);
        history = EditHistory::default();
        assert!(history.travel(current, true).is_none());
    }
    #[test]
    fn native_utf16_offsets_never_split_utf8_or_surrogate_pairs() {
        let text = "A🧭e\u{301}文";
        for (bytes, _) in text.char_indices() {
            assert_eq!(from_utf16(text, to_utf16(text, bytes)), bytes);
        }
        assert_eq!(from_utf16(text, 2), 5);
        assert_eq!(from_utf16(text, 999), text.len());
        assert_eq!(to_utf16(text, text.len()), 6);
    }
    #[test]
    fn navigation_and_deletion_preserve_combining_and_emoji_clusters() {
        let text = "e\u{301}👩‍👩‍👧‍👧x";
        let first = next_grapheme(text, 0);
        assert_eq!(&text[..first], "e\u{301}");
        let second = next_grapheme(text, first);
        assert_eq!(&text[first..second], "👩‍👩‍👧‍👧");
        assert_eq!(previous_grapheme(text, second), first);
        assert_eq!(previous_grapheme(text, 0), 0);
    }
    #[test]
    fn old_painted_offsets_cannot_split_current_utf8() {
        // Byte 2 was valid in "ABCD"; the current text has an emoji there.
        let current = "A🧭B";
        assert!(!replacement_fits(current, &(2..2), "x", 4096, true));
        assert!(!replacement_fits(current, &(1..3), "", 4096, true));
        assert!(replacement_fits(current, &(1..5), "x", 4096, true));
        assert_eq!(current_boundary(current, 2), 1);
        assert_eq!(current_boundary(current, usize::MAX), current.len());
        for offset in 0..=current.len() + 1 {
            assert!(current.is_char_boundary(current_boundary(current, offset)));
        }
    }
    #[test]
    fn rejected_paste_and_ime_replacement_cannot_exceed_budget() {
        assert!(replacement_fits("123456", &(0..6), "文🧭", 7, false));
        assert!(!replacement_fits("123456", &(0..6), "文🧭", 6, false));
        assert!(!replacement_fits("123456", &(0..0), "x", 6, false));
        assert!(!replacement_fits(
            "123456",
            &(0..6),
            "line\nline",
            30,
            false
        ));
        assert!(!replacement_fits("123456", &(0..99), "x", 30, false));
        assert!(replacement_fits("123456", &(0..6), "line\nline", 9, true));
        assert!(!replacement_fits("123456", &(0..6), "line\nline", 8, true));
        assert!(!replacement_fits("", &(0..0), "\0", 4096, true));
    }
}
