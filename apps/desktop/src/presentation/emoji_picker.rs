//! The built-in emoji picker: categories and a grid, shown inline above the composer
//! (or in the reaction card) instead of the system's floating palette. It works the
//! same on every platform, needs no focus hand-over and inserts at the caret.
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
                .max_h(px(208.))
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

    /// The picker above one composer, when that composer asked for it.
    pub(in crate::presentation) fn composer_emoji_card_on(
        &self,
        thread: bool,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (context, in_thread) = self.emoji_panel?;
        if in_thread != thread
            || self.snapshot.selected != Some(context)
            || thread && self.overlay != Some(Overlay::Thread)
        {
            return None;
        }
        Some(
            self.composer_card()
                .p_3()
                .gap_2()
                .child(self.emoji_grid(cx))
                .into_any_element(),
        )
    }

    /// The picker inside the reaction card.
    pub(in crate::presentation) fn reaction_emoji_grid(&self, cx: &Context<Self>) -> Div {
        self.emoji_grid(cx)
    }
}
