//! The built-in emoji picker: categories and a grid in a popover. It opens at the
//! composer's emoji button, or at the pointer for a reaction, instead of the
//! system's floating palette. It works the same on every platform, needs no focus
//! hand-over and inserts at the caret.
use super::controls::Availability;
use super::*;

mod data;

impl Desktop {
    /// Open or close the picker of one composer.
    pub(super) fn toggle_inline_emoji(&mut self, context: ConversationRef, thread: bool) {
        self.emoji_panel = if self.emoji_panel == Some((context, thread)) {
            None
        } else {
            Some((context, thread))
        };
    }

    pub(super) fn emoji_category(&mut self, index: usize) {
        if index < data::CATEGORIES.len() + 1 {
            self.emoji_category = index;
        }
    }

    /// An emoji was chosen: react with it, or put it into the open composer.
    pub(super) fn pick_emoji(
        &mut self,
        emoji: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pick_reaction_emoji(emoji, window, cx) {
            return;
        }
        let Some((context, thread)) = self.emoji_panel else {
            return;
        };
        if self.snapshot.selected != Some(context) || !self.composer_available() {
            return;
        }
        let editor = if thread {
            self.thread_composer.clone()
        } else {
            self.composer.clone()
        };
        editor.update(cx, |input, cx| input.insert_emoji(emoji, cx));
        self.emoji_recent.retain(|recent| *recent != emoji);
        self.emoji_recent.insert(0, emoji);
        self.emoji_recent.truncate(16);
        editor.focus_handle(cx).focus(window);
        cx.notify();
    }

    /// Category tabs (with the recent ones first, once there are any) over a grid.
    fn emoji_grid(&self, cx: &Context<Self>) -> Div {
        let t = self.theme;
        let has_recent = !self.emoji_recent.is_empty();
        // Tab 0 is "recent" when it exists; the others follow the data order.
        let shown = if has_recent {
            self.emoji_category.min(data::CATEGORIES.len())
        } else {
            self.emoji_category.max(1).min(data::CATEGORIES.len())
        };
        let mut tabs = self.segmented().flex_wrap().gap(px(2.));
        for index in usize::from(!has_recent)..=data::CATEGORIES.len() {
            let glyph = if index == 0 {
                "🕘"
            } else {
                data::CATEGORIES[index - 1].0
            };
            tabs = tabs.child(
                self.button(
                    gpui::ElementId::from(gpui::SharedString::from(format!(
                        "emoji-category-{index}"
                    ))),
                    glyph,
                    ButtonStyle::Quiet,
                    Availability::available(),
                    LocalAction::EmojiCategory(index),
                    cx,
                )
                .size(px(32.))
                .p_0()
                .rounded(px(7.))
                .justify_center()
                .text_size(self.sp(17.))
                .when(shown == index, |d| d.bg(t.selected)),
            );
        }
        let emojis: Vec<&'static str> = if shown == 0 {
            self.emoji_recent.clone()
        } else {
            data::CATEGORIES[shown - 1].1.to_vec()
        };
        div().flex().flex_col().gap_2().child(tabs).child(
            div()
                .id("emoji-grid")
                .max_h(px(252.))
                .overflow_y_scroll()
                .flex()
                .flex_wrap()
                .gap(px(2.))
                .children(emojis.into_iter().map(|emoji| {
                    self.button(
                        gpui::ElementId::from(gpui::SharedString::from(format!(
                            "emoji-{shown}-{emoji}"
                        ))),
                        emoji,
                        ButtonStyle::Quiet,
                        Availability::available(),
                        LocalAction::EmojiPick(emoji),
                        cx,
                    )
                    .size(px(38.))
                    .p_0()
                    .rounded(px(8.))
                    .justify_center()
                    .text_size(self.sp(22.))
                })),
        )
    }

    /// Records where one composer's emoji button is, so the popover can open there.
    pub(in crate::presentation) fn emoji_anchor(&self, thread: bool) -> impl IntoElement {
        let cell = self.emoji_anchor[usize::from(thread)].clone();
        gpui::canvas(move |bounds, _, _| cell.set(Some(bounds)), |_, _, _, _| {})
            .absolute()
            .inset_0()
    }

    fn emoji_popover_open(&self) -> bool {
        self.reaction_request().is_some() || self.emoji_panel.is_some()
    }

    fn close_emoji_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reaction_request().is_some() {
            self.close_reaction_capture(window, cx);
        } else {
            self.emoji_panel = None;
        }
        cx.notify();
    }

    /// Escape closes the popover before anything behind it sees the key.
    pub(super) fn emoji_popover_key(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.key != "escape" || !self.emoji_popover_open() {
            return false;
        }
        self.close_emoji_popover(window, cx);
        true
    }

    /// The picker as one root-layer popover: above the emoji button of the composer
    /// that asked for it, or where the pointer was when a reaction was requested.
    pub(super) fn emoji_popover(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let current = |context: ConversationRef, thread: bool| {
            self.snapshot.selected == Some(context)
                && (!thread || self.overlay == Some(Overlay::Thread))
        };
        let position = if let Some((context, thread)) = self.reaction_request() {
            if !current(context, thread) {
                return None;
            }
            self.emoji_point?
        } else {
            let (context, thread) = self.emoji_panel?;
            if !current(context, thread) {
                return None;
            }
            let button = self.emoji_anchor[usize::from(thread)].get()?;
            gpui::point(button.left(), button.top() - px(8.))
        };
        let anchors = self.emoji_anchor.clone();
        let panel = self
            .context_menu_surface()
            .id("emoji-popover")
            .occlude()
            .w(self.sp(346.))
            .p_2()
            .on_mouse_down_out(cx.listener(
                move |this, event: &gpui::MouseDownEvent, window, cx| {
                    // The emoji button toggles the popover itself.
                    let on_button = this.reaction_request().is_none()
                        && anchors.iter().any(|anchor| {
                            anchor
                                .get()
                                .is_some_and(|bounds| bounds.contains(&event.position))
                        });
                    if !on_button {
                        this.close_emoji_popover(window, cx);
                    }
                },
            ))
            .child(self.emoji_grid(cx));
        Some(
            gpui::deferred(
                gpui::anchored()
                    .position(position)
                    .anchor(gpui::Corner::BottomLeft)
                    .snap_to_window_with_margin(px(12.))
                    .child(panel),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }
}
