//! Reply and edit state belongs to the thread editor, independently of the timeline draft.
use super::*;

pub(super) struct Edit {
    pub context: ConversationRef,
    pub root: u128,
    pub message: u128,
    pub generation: u64,
    pub body: Text<4096>,
    pub pending: Option<super::super::state::PendingEdit>,
}

impl Desktop {
    pub(in crate::presentation) fn thread_composer_context(
        &self,
        cx: &Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let Some((context, root)) = self.thread_parent else {
            return Vec::new();
        };
        let target = self.thread_drafts.target(context);
        let editing = self.thread_edit_active(context, root);
        let mut rows = Vec::new();
        if editing || target.reply_to.is_some() {
            let preview = if editing {
                self.hint(self.l("Nachricht bearbeiten"))
            } else if let Some(message) =
                target.reply_to.and_then(|id| self.thread_row(context, id))
            {
                self.quote_preview(message.author.as_str(), Some(message.body.as_str()))
            } else {
                self.hint(self.l("Ursprüngliche Nachricht nicht mehr verfügbar."))
            };
            rows.push(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .p_2()
                    .rounded(px(8.))
                    .bg(self.theme.subtle)
                    .child(preview.flex_1())
                    .child(self.icon_button(
                        "thread-cancel-compose",
                        ("x", "Abbrechen"),
                        ButtonStyle::Quiet,
                        super::Availability::when(!self.thread_edit_pending(), ""),
                        LocalAction::MessageIn(context, Action::ThreadCancel),
                        cx,
                    ))
                    .into_any_element(),
            );
        }
        let show_mentions = self.thread_composer_active
            && self
                .message_ui
                .mention_ready
                .as_ref()
                .is_some_and(|(thread, query)| {
                    *thread && self.thread_composer.read(cx).mention_query() == Some(query.as_str())
                });
        if show_mentions
            && let Some(panel) = self
                .snapshot
                .message_panel
                .as_ref()
                .filter(|panel| panel.context == context)
        {
            for (index, mention) in panel.mentions.as_slice().iter().take(8).enumerate() {
                rows.push(
                    self.button(
                        ("thread-mention-choice", index),
                        mention.label.as_str().to_owned(),
                        ButtonStyle::Quiet,
                        true,
                        LocalAction::MessageIn(
                            context,
                            Action::InsertThreadMention(mention.identifier.as_str().to_owned()),
                        ),
                        cx,
                    )
                    .into_any_element(),
                );
            }
        }
        rows
    }
    pub(super) fn thread_can_reply(&self, context: ConversationRef, message: &MessageView) -> bool {
        reply_allowed(
            message,
            self.snapshot.content,
            self.selected().map_or(Decision::Unknown, |room| room.send),
            self.message_feature_enabled(MessageFeature::Reply),
            self.local_message_intents(),
            self.thread_drafts.pending(context) || self.message_ui.thread_edit.is_some(),
        )
    }
    pub(in crate::presentation) fn has_thread_edit(
        &self,
        account: Option<talk_core::ui::AccountId>,
    ) -> bool {
        self.message_ui.thread_edit.as_ref().is_some_and(|edit| {
            account.is_none_or(|account| edit.context.account.account == account)
        })
    }

    pub(in crate::presentation) fn retain_thread_edit(&mut self) {
        if self.message_ui.thread_edit.as_ref().is_some_and(|edit| {
            !self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .any(|account| account.scope == edit.context.account)
        }) {
            self.message_ui.thread_edit = None;
        }
    }

    pub(in crate::presentation) fn thread_row(
        &self,
        context: ConversationRef,
        id: u128,
    ) -> Option<&MessageView> {
        let (_, root) = self.thread_parent.filter(|(room, _)| *room == context)?;
        let panel = self
            .snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context && panel.thread_root == Some(root))?;
        panel
            .thread_root_message
            .iter()
            .chain(panel.thread_messages.as_slice())
            .chain(
                self.snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .filter(|row| row.local_id == root),
            )
            .find(|row| row.local_id == id)
    }

    pub(in crate::presentation) fn thread_edit_active(
        &self,
        context: ConversationRef,
        root: u128,
    ) -> bool {
        self.message_ui
            .thread_edit
            .as_ref()
            .is_some_and(|edit| edit.context == context && edit.root == root)
    }
    pub(in crate::presentation) fn thread_edit_pending(&self) -> bool {
        self.message_ui
            .thread_edit
            .as_ref()
            .is_some_and(|edit| edit.pending.is_some())
    }

    pub(super) fn thread_edit_available(
        &self,
        context: ConversationRef,
        root: u128,
        cx: &gpui::App,
    ) -> bool {
        let Some(edit) = self
            .message_ui
            .thread_edit
            .as_ref()
            .filter(|edit| edit.context == context && edit.root == root)
        else {
            return false;
        };
        self.overlay == Some(Overlay::Thread)
            && self.snapshot.selected == Some(context)
            && edit.pending.is_none()
            && self.thread_row(context, edit.message).is_some_and(|row| {
                message_mutation_allowed(
                    row,
                    MessageFeature::Edit,
                    self.message_feature_enabled(MessageFeature::Edit),
                    self.local_message_intents(),
                )
            })
            && !self.thread_composer.read(cx).composing()
            && self
                .thread_composer
                .read(cx)
                .submitted_value()
                .is_some_and(|body| !body.trim().is_empty())
    }

    pub(super) fn edit_in_thread(
        &mut self,
        context: ConversationRef,
        id: u128,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((room, root)) = self.thread_parent else {
            return;
        };
        if room != context
            || self.overlay != Some(Overlay::Thread)
            || self.message_ui.thread_edit.is_some()
            || self.thread_drafts.pending(context)
        {
            return;
        }
        let Some(row) = self.thread_row(context, id).filter(|row| {
            message_mutation_allowed(
                row,
                MessageFeature::Edit,
                self.message_feature_enabled(MessageFeature::Edit),
                self.local_message_intents(),
            )
        }) else {
            return;
        };
        let body = row.body.clone();
        if !self.save_thread_draft(cx) {
            return;
        }
        let Some(generation) = self.message_ui.thread_edit_generation.checked_add(1) else {
            return;
        };
        if !self.thread_composer.update(cx, |input, cx| {
            let loaded = input.set_context_value(body.as_str());
            cx.notify();
            loaded
        }) {
            return;
        }
        self.message_ui.thread_edit_generation = generation;
        self.message_ui.thread_edit = Some(Edit {
            context,
            root,
            message: id,
            generation,
            body,
            pending: None,
        });
        self.thread_composer.focus_handle(cx).focus(window);
    }

    pub(super) fn reply_in_thread(
        &mut self,
        context: ConversationRef,
        id: u128,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((room, root)) = self.thread_parent else {
            return;
        };
        if room != context
            || self.overlay != Some(Overlay::Thread)
            || self.thread_edit_active(context, root)
        {
            return;
        }
        let allowed = self.thread_row(context, id).is_some_and(|row| {
            reply_allowed(
                row,
                self.snapshot.content,
                self.selected().map_or(Decision::Unknown, |room| room.send),
                self.message_feature_enabled(MessageFeature::Reply),
                self.local_message_intents(),
                self.thread_drafts.pending(context),
            )
        });
        if !allowed || !self.save_thread_draft(cx) {
            return;
        }
        let previous = self.thread_drafts.target(context);
        if !self.thread_drafts.set_target(
            context,
            ComposeTarget {
                thread: Some(root),
                reply_to: Some(id),
                ..Default::default()
            },
        ) {
            return;
        }
        let body = self.thread_drafts.text(context).to_owned();
        if !self.thread_composer.update(cx, |input, cx| {
            let loaded = input.set_context_value(&body);
            cx.notify();
            loaded
        }) {
            self.thread_drafts.set_target(context, previous);
            return;
        }
        self.thread_composer.focus_handle(cx).focus(window);
    }

    pub(super) fn cancel_thread_compose(
        &mut self,
        context: ConversationRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((room, root)) = self.thread_parent else {
            return;
        };
        if room != context {
            return;
        }
        if self.thread_edit_active(context, root) {
            if self
                .message_ui
                .thread_edit
                .as_ref()
                .is_some_and(|edit| edit.pending.is_some())
            {
                return;
            }
            self.message_ui.thread_edit = None;
        } else {
            if !self.save_thread_draft(cx) {
                return;
            }
            let mut target = self.thread_drafts.target(context);
            target.reply_to = None;
            if !self.thread_drafts.set_target(context, target) {
                return;
            }
        }
        let body = self.thread_drafts.text(context).to_owned();
        self.thread_composer.update(cx, |input, cx| {
            input.set_context_value(&body);
            cx.notify();
        });
        self.thread_composer.focus_handle(cx).focus(window);
    }

    pub(super) fn submit_thread_edit(
        &mut self,
        context: ConversationRef,
        root: u128,
        cx: &mut Context<Self>,
    ) {
        if !self.thread_edit_available(context, root, cx) || !self.save_thread_draft(cx) {
            return;
        }
        let Some(edit) = self.message_ui.thread_edit.as_ref() else {
            return;
        };
        let message = edit.message;
        let generation = edit.generation;
        let body = edit.body.clone();
        let editor_generation = self.thread_composer.read(cx).edit_generation();
        if let Some(request) = self.submit_message(MessageAction::Edit {
            message,
            body: body.clone(),
        }) && let Some(edit) = self.message_ui.thread_edit.as_mut()
        {
            edit.pending = Some(super::super::state::PendingEdit {
                request,
                message,
                generation,
                editor_generation,
                body,
            });
        }
        cx.notify();
    }

    pub(super) fn complete_thread_edit(
        &mut self,
        request: RequestId,
        completion: Completion,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = self.message_ui.thread_edit.as_mut() else {
            return;
        };
        let active = self.thread_parent == Some((edit.context, edit.root))
            && self.overlay == Some(Overlay::Thread);
        let input = self.thread_composer.read(cx);
        let body = if active {
            input.committed_value()
        } else {
            edit.body.as_str()
        };
        let editor_generation = active.then(|| input.edit_generation());
        let unchanged = if active {
            super::super::state::PendingEdit::complete(
                &mut edit.pending,
                request,
                completion,
                Some((edit.message, edit.generation)),
                body,
                (!input.composing()).then_some(editor_generation.unwrap_or_default()),
            )
        } else {
            let pending = edit
                .pending
                .as_ref()
                .filter(|pending| pending.request == request);
            let restore = pending.is_some_and(|pending| {
                matches!(completion, Completion::Queued | Completion::ServerAccepted)
                    && pending.body.as_str() == body
            });
            if pending.is_some() {
                edit.pending = None;
                Some(restore)
            } else {
                None
            }
        };
        if unchanged == Some(true) {
            let context = edit.context;
            self.message_ui.thread_edit = None;
            if active {
                let body = self.thread_drafts.text(context).to_owned();
                self.thread_composer.update(cx, |input, cx| {
                    input.set_context_value(&body);
                    cx.notify();
                });
            }
        }
    }
}
