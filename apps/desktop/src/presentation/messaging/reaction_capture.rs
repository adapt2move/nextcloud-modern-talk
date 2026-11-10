//! Reactions beyond the quick grid: a card with the built-in emoji picker. The
//! emoji picked reacts at once and the card closes.
use super::*;

impl Desktop {
    pub(super) fn open_reaction_capture(
        &mut self,
        context: ConversationRef,
        message: u128,
        thread: bool,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context) || !self.reaction_available(message).is_enabled()
        {
            return;
        }
        self.message_ui.reaction_capture = Some((context, message, thread));
        cx.notify();
    }

    /// A picked emoji reacts when a reaction card is open. False: none is.
    pub(in crate::presentation) fn pick_reaction_emoji(
        &mut self,
        emoji: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((_, message, _)) = self.message_ui.reaction_capture else {
            return false;
        };
        if let Ok(text) = Text::new(emoji) {
            self.toggle_reaction(message, text);
        }
        self.close_reaction_capture(window, cx);
        true
    }

    pub(super) fn close_reaction_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, _, thread)) = self.message_ui.reaction_capture.take() else {
            return;
        };
        self.retire_emoji_picker();
        let root = thread
            .then(|| self.thread_parent.map(|(_, root)| root))
            .flatten();
        self.restore_deadline_focus(root, window, cx);
        cx.notify();
    }

    /// Runs every frame: the card closes when its room or message is gone.
    pub(super) fn poll_reaction_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((context, message, _)) = self.message_ui.reaction_capture else {
            return;
        };
        if self.snapshot.selected != Some(context) || self.reaction_message(message).is_none() {
            self.close_reaction_capture(window, cx);
        }
    }

    pub(in crate::presentation) fn reaction_capture_card_on(
        &self,
        thread: bool,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (context, _, in_thread) = self.message_ui.reaction_capture?;
        if in_thread != thread
            || self.snapshot.selected != Some(context)
            || in_thread && self.overlay != Some(Overlay::Thread)
        {
            return None;
        }
        Some(
            self.composer_card()
                .p_3()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .font_weight(FontWeight::MEDIUM)
                                .child(self.l("Reaktion hinzufügen")),
                        )
                        .child(self.icon_button(
                            "cancel-reaction-capture",
                            ("x", "Abbrechen"),
                            ButtonStyle::Quiet,
                            Availability::available(),
                            LocalAction::MessageIn(context, Action::CancelReactionCapture),
                            cx,
                        )),
                )
                .child(self.reaction_emoji_grid(cx))
                .into_any_element(),
        )
    }
}
