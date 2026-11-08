//! Reactions beyond the quick grid come from the system's own emoji picker. A
//! small field receives the chosen character; the first emoji in it reacts.
use super::*;
use crate::presentation::controls::BannerRole;
use unicode_segmentation::UnicodeSegmentation;

/// The first emoji in `text`. Letters, digits and punctuation never qualify,
/// so typing into the field cannot react by accident.
fn emoji_grapheme(text: &str) -> Option<Text<64>> {
    text.graphemes(true)
        .find(|grapheme| {
            !grapheme.is_ascii()
                && grapheme
                    .chars()
                    .any(|c| u32::from(c) >= 0x2190 && !c.is_alphanumeric())
        })
        .and_then(|grapheme| Text::new(grapheme).ok())
}

impl Desktop {
    pub(super) fn open_reaction_capture(
        &mut self,
        context: ConversationRef,
        message: u128,
        thread: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context) || !self.reaction_available(message).is_enabled()
        {
            return;
        }
        self.message_ui.reaction_capture = Some((context, message, thread));
        self.message_ui.reaction_error = None;
        self.message_ui.reaction_input.update(cx, |input, cx| {
            input.reset();
            cx.notify();
        });
        self.message_ui.reaction_input.focus_handle(cx).focus(window);
        self.show_reaction_picker(window, cx);
        cx.notify();
    }

    pub(super) fn close_reaction_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, _, thread)) = self.message_ui.reaction_capture.take() else {
            return;
        };
        self.message_ui.reaction_error = None;
        self.retire_emoji_picker();
        let root = thread.then(|| self.thread_parent.map(|(_, root)| root)).flatten();
        self.restore_deadline_focus(root, window, cx);
        cx.notify();
    }

    /// Open the platform picker for the focused reaction field. Wait one frame
    /// so the native text handler owns focus before the picker inserts text.
    pub(super) fn show_reaction_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((context, ..)) = self.message_ui.reaction_capture else {
            return;
        };
        if !cfg!(target_os = "macos") && !self.settings_port.emoji_picker_available() {
            self.message_ui.reaction_error =
                Some("Die native Emoji-Auswahl ist auf diesem System nicht verfügbar.");
            return;
        }
        self.message_ui.reaction_input.focus_handle(cx).focus(window);
        let view = cx.weak_entity();
        window.on_next_frame(move |window, cx| {
            let _ = view.update(cx, |this, cx| {
                let field = this.message_ui.reaction_input.clone();
                if this.message_ui.reaction_capture.map(|(room, ..)| room) != Some(context)
                    || !window.is_window_active()
                    || !field.focus_handle(cx).is_focused(window)
                {
                    return;
                }
                if cfg!(target_os = "macos") {
                    window.show_character_palette();
                } else {
                    this.emoji_picker_context = None;
                    match this.settings_port.show_emoji_picker() {
                        Ok(()) => this.emoji_picker_context = Some(context),
                        Err(error) => this.message_ui.reaction_error = Some(error.label()),
                    }
                    cx.notify();
                }
            });
        });
    }

    /// Runs every frame: once the field holds an emoji and no composition is in
    /// progress, that emoji is the reaction.
    pub(super) fn poll_reaction_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((context, message, _)) = self.message_ui.reaction_capture else {
            return;
        };
        if self.snapshot.selected != Some(context) || self.reaction_message(message).is_none() {
            self.close_reaction_capture(window, cx);
            return;
        }
        let input = self.message_ui.reaction_input.read(cx);
        if input.composing() {
            return;
        }
        let Some(emoji) = emoji_grapheme(&input.content) else {
            return;
        };
        self.toggle_reaction(message, emoji);
        self.close_reaction_capture(window, cx);
    }

    pub(in crate::presentation) fn reaction_capture_card_on(
        &self,
        thread: bool,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let (context, message, in_thread) = self.message_ui.reaction_capture?;
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
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            self.input_surface(&self.message_ui.reaction_input, cx)
                                .flex_1()
                                .child(self.message_ui.reaction_input.clone()),
                        )
                        .child(self.button(
                            "open-reaction-picker",
                            "Emoji-Auswahl öffnen",
                            ButtonStyle::Secondary,
                            self.reaction_available(message),
                            LocalAction::MessageIn(context, Action::OpenReactionPicker),
                            cx,
                        )),
                )
                .when_some(self.message_ui.reaction_error, |d, error| {
                    d.child(self.banner(BannerRole::Error, self.l(error)))
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::emoji_grapheme;

    #[test]
    fn only_emoji_become_reactions() {
        for (text, expected) in [
            ("👍", Some("👍")),
            ("x🎉y", Some("🎉")),
            ("❤️", Some("❤️")),
            ("👨‍👩‍👧", Some("👨‍👩‍👧")),
            ("🇩🇪", Some("🇩🇪")),
            ("ä ß 日本", None),
            ("abc 123 !?", None),
            ("", None),
        ] {
            assert_eq!(
                emoji_grapheme(text).as_ref().map(|emoji| emoji.as_str()),
                expected,
                "{text}"
            );
        }
    }
}
