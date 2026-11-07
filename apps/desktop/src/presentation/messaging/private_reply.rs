//! Private replies keep the displayed quote scoped to one source and destination.
use super::*;
use crate::presentation::feedback::Feedback;
use talk_core::ui::conversation::CreateAction;
use talk_core::ui::messaging::PrivateReplyTarget;

pub(super) struct Pending {
    request: RequestId,
    origin: ConversationRef,
    target: PrivateReplyTarget,
}

impl Desktop {
    pub(super) fn private_reply_available(&self, message: &MessageView) -> bool {
        let Some(room) = self.selected() else {
            return false;
        };
        self.snapshot.content == ContentState::Ready
            && self.message_feature_enabled(MessageFeature::PrivateReply)
            && room.navigation_key.is_some()
            && room.draft_clipboard == Decision::Allowed
            && !self.drafts.pending(room.context)
            && self.message_ui.editing.is_none()
            && self.message_ui.pending_edit.is_none()
            && !message.own
            && !message.metadata.deleted
            && !message.metadata.system
            && message.metadata.replyable
            && message
                .metadata
                .author_key
                .as_ref()
                .is_some_and(|key| key.as_str().starts_with("5:users:"))
    }

    pub(super) fn begin_private_reply(
        &mut self,
        context: ConversationRef,
        message: u128,
        cx: &mut Context<Self>,
    ) {
        if Some(context) != self.snapshot.selected
            || self.message_ui.private_reply_pending.is_some()
        {
            return;
        }
        let Some(row) = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .find(|row| row.local_id == message && self.private_reply_available(row))
        else {
            return;
        };
        let Some(source) = self.selected().and_then(|room| room.navigation_key.clone()) else {
            return;
        };
        let body = if row.body.as_str().is_empty() {
            row.attachments
                .as_slice()
                .first()
                .map_or("", |file| file.name.as_str())
        } else {
            row.body.as_str()
        };
        let Some(body) = excerpt(body) else { return };
        let target = PrivateReplyTarget {
            source,
            message,
            author: row.author.clone(),
            body,
        };
        if !self.save_draft(cx) {
            return;
        }
        let Some(request) = self.room_ui.request.checked_add(1) else {
            return;
        };
        self.room_ui.request = request;
        let request = RequestId(request);
        if self.submit(UiIntent::Create {
            account: context.account,
            request,
            action: CreateAction::PrivateReply { context, message },
        }) {
            self.message_ui.private_reply_pending = Some(Pending {
                request,
                origin: context,
                target,
            });
        }
    }

    pub(in crate::presentation) fn refresh_private_reply(&mut self) {
        let Some(pending) = self.message_ui.private_reply_pending.as_ref() else {
            return;
        };
        let result = self
            .snapshot
            .create_panel
            .as_ref()
            .filter(|panel| panel.account == pending.origin.account)
            .and_then(|panel| panel.result)
            .filter(|result| result.request == pending.request);
        let Some(result) = result else {
            if self.snapshot.selected != Some(pending.origin) {
                self.message_ui.private_reply_pending = None;
            }
            return;
        };
        let Some(pending) = self.message_ui.private_reply_pending.take() else {
            return;
        };
        if result.completion == Completion::ServerAccepted
            && let Some(destination) = result.conversation
            && self.snapshot.selected == Some(destination)
            && destination.account == pending.origin.account
        {
            if self.drafts.set_target(
                destination,
                ComposeTarget {
                    private_reply: Some(pending.target),
                    ..Default::default()
                },
            ) {
                self.message_ui.private_reply_focus = Some(destination);
            } else {
                self.feedback = Some(
                    Feedback::error("Privater Entwurf konnte nicht geöffnet werden.")
                        .in_conversation(destination),
                );
            }
        } else if self.snapshot.selected == Some(pending.origin)
            && result.completion != Completion::ServerAccepted
        {
            self.feedback = Some(
                Feedback::error(
                    "Privates Gespräch konnte nicht geöffnet werden. Bitte erneut versuchen.",
                )
                .in_conversation(pending.origin),
            );
        }
        if result.completion != Completion::ServerAccepted {
            self.submit(UiIntent::AcknowledgeCreation {
                account: pending.origin.account,
                request: pending.request,
            });
        }
    }

    pub(super) fn private_reply_preview(
        &self,
        context: ConversationRef,
        target: &PrivateReplyTarget,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .pl_3()
            .pr_1()
            .py_2()
            .rounded(px(8.))
            .bg(self.theme.subtle)
            .child(
                self.quote_preview(target.author.as_str(), Some(target.body.as_str()))
                    .flex_1(),
            )
            .child(self.icon_button(
                "cancel-compose-mode",
                ("x", "Abbrechen"),
                ButtonStyle::Quiet,
                super::Availability::available(),
                LocalAction::MessageIn(context, Action::CancelCompose),
                cx,
            ))
    }
}

fn excerpt(value: &str) -> Option<Text<512>> {
    let end = value
        .char_indices()
        .take_while(|(index, ch)| index + ch.len_utf8() <= 512)
        .last()
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    Text::new(&value[..end]).ok()
}

#[cfg(test)]
mod tests {
    use super::excerpt;

    #[test]
    fn preview_is_bounded_without_splitting_unicode() {
        let value = format!("{}💬tail", "a".repeat(510));
        assert_eq!(excerpt(&value).unwrap().as_str(), "a".repeat(510));
        assert_eq!(excerpt("").unwrap().as_str(), "");
    }
}
