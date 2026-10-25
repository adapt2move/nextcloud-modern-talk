//! Direct reactions share the message owner's admission and reconciliation.
use super::*;

// A finite Unicode palette, not a downloaded emoji catalog or custom-emoji claim.
pub(super) const EMOJI: [&str; 36] = [
    "👍", "❤️", "😂", "🎉", "👏", "🙏", "👀", "✅", "🙌", "🔥", "💯", "😄", "🙂", "😊", "😍", "🤔",
    "😢", "😮", "🚀", "💪", "🤝", "👎", "😅", "🤣", "🥳", "😎", "💡", "🫡", "🤗", "😴", "☕", "⭐",
    "❓", "⚠️", "🎯", "🛠️",
];

fn quick_reactions(recent: &[Text<64>]) -> Vec<Text<64>> {
    use unicode_segmentation::UnicodeSegmentation;
    let mut result = Vec::with_capacity(3);
    for emoji in recent.iter().take(12).cloned().chain(
        EMOJI[..3]
            .iter()
            .map(|emoji| Text::new(emoji).expect("fixed emoji")),
    ) {
        if !emoji.as_str().is_ascii()
            && emoji.as_str().graphemes(true).count() == 1
            && !result.contains(&emoji)
        {
            result.push(emoji);
            if result.len() == 3 {
                break;
            }
        }
    }
    result
}

fn reaction_command(
    message: &MessageView,
    emoji: Text<64>,
    allowed: bool,
) -> Option<MessageAction> {
    if !allowed || !message_actions_allowed(message) {
        return None;
    }
    let own = message
        .metadata
        .reactions
        .as_slice()
        .iter()
        .any(|reaction| reaction.emoji == emoji && reaction.own);
    Some(MessageAction::React {
        message: message.local_id,
        emoji,
        add: !own,
    })
}

impl Desktop {
    pub(super) fn reaction_picker_button(
        &self,
        message: &MessageView,
        source: menu::Source,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let context = self.snapshot.selected.expect("message context");
        let available = self.reaction_available(message.local_id);
        let open = self
            .message_ui
            .menu
            .anchored_to(context, message.local_id, source);
        self.icon_button(
            scoped_id(source.key(), context, message.local_id, 0),
            ("smile", "Reaktion hinzufügen"),
            ButtonStyle::Quiet,
            if open {
                available.explained_elsewhere()
            } else {
                available
            },
            LocalAction::MessageIn(context, Action::ReactionPalette(message.local_id, source)),
            cx,
        )
        .bg(if open {
            self.theme.selected
        } else {
            self.theme.surface
        })
        .child(
            self.scaled_icon("plus", 9., self.theme.muted)
                .absolute()
                .right(px(3.))
                .bottom(px(3.)),
        )
        .child(self.message_menu_anchor(context, message.local_id, source))
    }

    pub(super) fn quick_reaction_button(
        &self,
        message: &MessageView,
        index: usize,
        cx: &Context<Self>,
    ) -> gpui::Stateful<Div> {
        let context = self.snapshot.selected.expect("message context");
        let quick = quick_reactions(self.preferences.active.recent_reactions.as_slice());
        let fallback = Text::new(EMOJI[0]).expect("fixed emoji");
        let emoji = quick.get(index).unwrap_or(&fallback);
        let own = message
            .metadata
            .reactions
            .as_slice()
            .iter()
            .any(|reaction| reaction.emoji == *emoji && reaction.own);
        self.button(
            gpui::ElementId::Name(
                format!(
                    "{}:{}",
                    scoped_id("message-quick-reaction", context, message.local_id, 0),
                    emoji.as_str()
                )
                .into(),
            ),
            emoji.as_str().to_owned(),
            ButtonStyle::Quiet,
            self.reaction_available(message.local_id),
            LocalAction::MessageIn(context, Action::React(message.local_id, emoji.clone())),
            cx,
        )
        .size(px(34.))
        .min_h(px(34.))
        .p_0()
        .bg(if own {
            self.theme.selected
        } else {
            self.theme.surface
        })
        .text_size(self.sp(17.))
    }

    pub(super) fn reaction_available(&self, message: u128) -> Availability {
        let available = self.message_available(MessageFeature::React);
        if available.is_enabled()
            && !self
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|row| row.local_id == message && message_actions_allowed(row))
        {
            Availability::when(false, "Reaktionen sind derzeit nicht verfügbar.")
        } else {
            available
        }
    }

    pub(super) fn toggle_reaction(&mut self, message: u128, emoji: Text<64>) -> bool {
        let recent = emoji.clone();
        let allowed = self.reaction_available(message).is_enabled();
        let command = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|row| row.local_id == message)
            .and_then(|row| reaction_command(row, emoji, allowed));
        let Some(command) = command else {
            return false;
        };
        let add = matches!(&command, MessageAction::React { add: true, .. });
        if self.submit_message(command).is_some() {
            if add {
                self.remember_reaction_emoji(recent);
            }
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use talk_core::ui::{Page, messaging::Reaction};
    fn message() -> MessageView {
        MessageView {
            local_id: 7,
            author: Text::new("Fictional").unwrap(),
            initials: Text::new("F").unwrap(),
            time_label: Text::new("10:00").unwrap(),
            body: Text::new("Fixture").unwrap(),
            own: false,
            delivery: talk_core::ui::presentation::DeliveryState::Received,
            attachments: Page::empty(),
            reply_count: 0,
            metadata: Default::default(),
        }
    }
    #[test]
    fn reaction_toggle_uses_current_own_state_and_requires_live_permission() {
        let mut row = message();
        let emoji = Text::new("👍").unwrap();
        assert!(matches!(
            reaction_command(&row, emoji.clone(), true),
            Some(MessageAction::React { add: true, .. })
        ));
        row.metadata
            .reactions
            .try_push(Reaction {
                emoji: emoji.clone(),
                count: 2,
                own: true,
            })
            .unwrap();
        assert!(matches!(
            reaction_command(&row, emoji.clone(), true),
            Some(MessageAction::React { add: false, .. })
        ));
        assert!(reaction_command(&row, emoji.clone(), false).is_none());
        row.metadata.deleted = true;
        assert!(reaction_command(&row, emoji.clone(), true).is_none());
        row.metadata.deleted = false;
        row.metadata.system = true;
        assert!(reaction_command(&row, emoji, true).is_none());
    }
    #[test]
    fn quick_reactions_prefer_recent_unicode_keys_without_duplicates_or_slot_identity() {
        let keys = ["🤖", "👍", "🤖"].map(|emoji| Text::new(emoji).unwrap());
        assert_eq!(
            quick_reactions(&keys)
                .iter()
                .map(Text::as_str)
                .collect::<Vec<_>>(),
            ["🤖", "👍", "❤️"]
        );
        assert_eq!(
            quick_reactions(&[])
                .iter()
                .map(Text::as_str)
                .collect::<Vec<_>>(),
            ["👍", "❤️", "😂"]
        );
        let invalid = ["custom-emoji", "👍👍"].map(|emoji| Text::new(emoji).unwrap());
        assert_eq!(
            quick_reactions(&invalid)
                .iter()
                .map(Text::as_str)
                .collect::<Vec<_>>(),
            ["👍", "❤️", "😂"]
        );
    }
}
