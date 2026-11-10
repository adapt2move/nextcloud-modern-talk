//! Reactions beyond the quick grid: the emoji popover opens at the pointer. The
//! emoji picked reacts at once and the popover closes.
use super::*;

impl Desktop {
    pub(super) fn open_reaction_capture(
        &mut self,
        context: ConversationRef,
        message: u128,
        thread: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.snapshot.selected != Some(context) || !self.reaction_available(message).is_enabled()
        {
            return;
        }
        self.emoji_panel = None;
        self.emoji_point = Some(window.mouse_position());
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

    /// The room and composer side of the open reaction request, if any.
    pub(in crate::presentation) fn reaction_request(&self) -> Option<(ConversationRef, bool)> {
        self.message_ui
            .reaction_capture
            .map(|(context, _, thread)| (context, thread))
    }

    pub(in crate::presentation) fn close_reaction_capture(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    /// Runs every frame: the popover closes when its room or message is gone.
    pub(super) fn poll_reaction_capture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((context, message, _)) = self.message_ui.reaction_capture else {
            return;
        };
        if self.snapshot.selected != Some(context) || self.reaction_message(message).is_none() {
            self.close_reaction_capture(window, cx);
        }
    }
}
