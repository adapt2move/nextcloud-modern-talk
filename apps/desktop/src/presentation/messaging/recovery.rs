//! One failed-text record at a time. Restoring a copy never acknowledges or
//! deletes the owner's original; replacement and discard remain typed intents.
use super::*;
use talk_core::ui::messaging::{FailedDraft, FailedDraftKind};

#[derive(Clone, Copy)]
pub(crate) enum RecoveryAction {
    Previous(u128),
    Next(u128),
    Retry(u128),
    Restore(u128),
    Discard(u128),
}

fn selected(
    drafts: &[FailedDraft],
    selection: Option<(ConversationRef, u128)>,
    context: ConversationRef,
) -> Option<(usize, &FailedDraft)> {
    let index = selection
        .filter(|(room, _)| *room == context)
        .and_then(|(_, operation)| drafts.iter().position(|draft| draft.operation == operation))
        .unwrap_or(0);
    drafts.get(index).map(|draft| (index, draft))
}

fn next(drafts: &[FailedDraft], current: u128, forward: bool) -> Option<u128> {
    let index = drafts.iter().position(|draft| draft.operation == current)?;
    let index = if forward {
        (index + 1) % drafts.len()
    } else {
        (index + drafts.len() - 1) % drafts.len()
    };
    drafts.get(index).map(|draft| draft.operation)
}

fn restoration_target_available(
    drafts: &crate::presentation::state::Drafts,
    context: ConversationRef,
    target: &ComposeTarget,
    composer_loaded: bool,
) -> bool {
    // An empty field is not evidence of an empty stored draft after a rejected
    // context load. Neither matching targets nor a small recovery copy grant
    // permission to replace that unavailable retained content.
    composer_loaded
        && (drafts.target(context) == *target || !drafts.has_text_for_target(context, target))
}

impl Desktop {
    fn current_failed_drafts(&self, context: ConversationRef) -> Option<&[FailedDraft]> {
        if self.snapshot.selected != Some(context)
            || self.snapshot.active_account != Some(context.account)
        {
            return None;
        }
        self.snapshot
            .message_panel
            .as_ref()
            .filter(|panel| panel.context == context)
            .map(|panel| panel.failed_drafts.as_slice())
    }

    fn can_restore_failed_draft(
        &self,
        context: ConversationRef,
        draft: &FailedDraft,
        cx: &Context<Self>,
    ) -> bool {
        !draft.retrying
            && self.composer.read(cx).content.is_empty()
            && !self.composer.read(cx).composing()
            && self.message_ui.editing.is_none()
            && self.message_ui.pending_edit.is_none()
            && self.message_ui.time_target.is_none()
            && !self.drafts.pending(context)
            && restoration_target_available(
                &self.drafts,
                context,
                &draft.target,
                self.message_ui.composer_load_failed != Some(context),
            )
    }

    fn can_retry_failed_draft(&self, draft: &FailedDraft) -> bool {
        !draft.retrying
            && self.local_message_intents()
            && match draft.kind {
                FailedDraftKind::Send => self
                    .selected()
                    .is_some_and(|row| decision_allows_intent(row.send, true)),
                FailedDraftKind::Edit(message) => {
                    self.message_feature_enabled(MessageFeature::Edit)
                        && self
                            .snapshot
                            .messages
                            .as_slice()
                            .iter()
                            .find(|row| row.local_id == message)
                            .is_none_or(|row| {
                                !row.metadata.deleted && row.metadata.edit != Decision::Denied
                            })
                }
                FailedDraftKind::Schedule(_) => {
                    self.message_feature_enabled(MessageFeature::Schedules)
                }
            }
    }

    pub(super) fn recover_failed_draft(
        &mut self,
        context: ConversationRef,
        action: RecoveryAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let operation = match action {
            RecoveryAction::Previous(id)
            | RecoveryAction::Next(id)
            | RecoveryAction::Retry(id)
            | RecoveryAction::Restore(id)
            | RecoveryAction::Discard(id) => id,
        };
        let Some(drafts) = self.current_failed_drafts(context) else {
            return;
        };
        let Some((_, draft)) = selected(drafts, self.message_ui.failed_selection, context)
            .filter(|(_, draft)| draft.operation == operation)
        else {
            return;
        };
        match action {
            RecoveryAction::Previous(_) | RecoveryAction::Next(_) => {
                if let Some(operation) =
                    next(drafts, operation, matches!(action, RecoveryAction::Next(_)))
                {
                    self.message_ui.failed_selection = Some((context, operation));
                }
            }
            RecoveryAction::Retry(_) if self.can_retry_failed_draft(draft) => {
                self.submit_message(MessageAction::RetryQueued(operation));
            }
            RecoveryAction::Discard(_) if !draft.retrying => {
                self.submit_message(MessageAction::DiscardQueued(operation));
            }
            RecoveryAction::Restore(_) if self.can_restore_failed_draft(context, draft, cx) => {
                let draft = draft.clone();
                let generation = if matches!(draft.kind, FailedDraftKind::Edit(_)) {
                    let Some(generation) = self.message_ui.edit_generation.checked_add(1) else {
                        self.report_draft_error(talk_core::ui::SubmitError::Busy, context);
                        return;
                    };
                    Some(generation)
                } else {
                    None
                };
                let mut restored_drafts = self.drafts.clone();
                if !restored_drafts.set_target(context, draft.target.clone()) {
                    self.report_draft_error(talk_core::ui::SubmitError::Busy, context);
                    return;
                }
                if !matches!(draft.kind, FailedDraftKind::Edit(_))
                    && let Err(error) = restored_drafts.save(context, draft.body.as_str())
                {
                    self.report_draft_error(error, context);
                    return;
                }
                if !self.load_composer_value(draft.body.as_str(), cx) {
                    return;
                }
                self.drafts = restored_drafts;
                if let FailedDraftKind::Edit(message) = draft.kind {
                    self.message_ui.edit_generation =
                        generation.unwrap_or(self.message_ui.edit_generation);
                    self.message_ui.editing = Some(message);
                }
                if let FailedDraftKind::Schedule(timestamp) = draft.kind {
                    self.open_message_deadline(None, window, cx);
                    if let Some(local) = self.settings_port.local_datetime(timestamp) {
                        self.message_ui.time_input.update(cx, |input, cx| {
                            input.set_context_value(local.as_str());
                            cx.notify();
                        });
                    }
                } else {
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    pub(super) fn failed_draft_card(&self, cx: &Context<Self>) -> Option<gpui::AnyElement> {
        let context = self.snapshot.selected?;
        let drafts = self.current_failed_drafts(context)?;
        let (index, draft) = selected(drafts, self.message_ui.failed_selection, context)?;
        let operation = draft.operation;
        let action = |action| LocalAction::MessageIn(context, super::Action::Recovery(action));
        let copy_allowed = !self.background_inert
            && !self.projection_blocked
            && !self.update_restart_pending
            && self
                .selected()
                .is_some_and(|room| room.draft_clipboard == Decision::Allowed);
        let restore = self.can_restore_failed_draft(context, draft, cx);
        Some(self.composer_card().p_3().gap_2()
            .child(div().flex().items_center().gap_2()
                .child(div().flex_1().font_weight(FontWeight::MEDIUM).child(self.l(match draft.kind {
                    FailedDraftKind::Send => "Senden fehlgeschlagen",
                    FailedDraftKind::Edit(_) => "Änderung fehlgeschlagen",
                    FailedDraftKind::Schedule(_) => "Planung fehlgeschlagen",
                })))
                .when(drafts.len() > 1, |row| row
                    .child(self.icon_button(scoped_id("failed-previous", context, 0, 0), ("arrow-left", "Vorheriger Entwurf"), ButtonStyle::Quiet,
                        Availability::available(), action(RecoveryAction::Previous(operation)), cx))
                    .child(div().font_family("Google Sans Code").text_size(self.sp(11.)).child(format!("{}/{}", index + 1, drafts.len())))
                    .child(self.icon_button(scoped_id("failed-next", context, 0, 0), ("arrow-right", "Nächster Entwurf"), ButtonStyle::Quiet,
                        Availability::available(), action(RecoveryAction::Next(operation)), cx))))
            .child(div().max_h(self.sp(60.)).overflow_hidden().text_size(self.sp(13.)).line_height(self.sp(20.))
                .child(crate::presentation::selectable::SelectableText::new(scoped_id("failed-text", context, operation, 0), draft.body.as_str(), self.text_selection.clone(), self.theme, copy_allowed)))
            .child(div().flex().flex_wrap().gap_2()
                .child(self.button(scoped_id("failed-retry", context, operation, 0), "Erneut versuchen", ButtonStyle::Secondary,
                    Availability::when(self.can_retry_failed_draft(draft), "Diese Aktion kann gerade nicht erneut versucht werden."), action(RecoveryAction::Retry(operation)), cx))
                .child(self.button(scoped_id("failed-restore", context, operation, 0), "Als Entwurf bearbeiten", ButtonStyle::Quiet,
                    Availability::when(restore, "Beende zuerst die aktuelle Eingabe. Ein vorhandener Entwurf wird nicht überschrieben."), action(RecoveryAction::Restore(operation)), cx))
                .child(self.button(scoped_id("failed-discard", context, operation, 0), "Verwerfen", ButtonStyle::Quiet,
                    Availability::when(!draft.retrying, "Der Entwurf wird gerade verarbeitet."), action(RecoveryAction::Discard(operation)), cx)))
            .into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_preserves_an_unloaded_draft_until_the_user_can_deliberately_clear_it() {
        use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let mut drafts = crate::presentation::state::Drafts::default();
        let retained = "K".repeat(600);
        drafts.save(context, &retained).unwrap();
        let target = ComposeTarget::default();
        // A lower 512-byte server ceiling rejected the contextual load, so the
        // empty field must not authorize the failed record's small replacement.
        assert!(!restoration_target_available(
            &drafts, context, &target, false
        ));
        assert!(!restoration_target_available(
            &drafts,
            context,
            &ComposeTarget {
                reply_to: Some(42),
                ..Default::default()
            },
            false
        ));
        assert_eq!(drafts.text(context), retained);
        // After a successful load and deliberate clearing, recovery is allowed.
        drafts.save(context, "").unwrap();
        assert!(restoration_target_available(
            &drafts, context, &target, true
        ));
        let mut restored = drafts.clone();
        restored.save(context, "x").unwrap();
        drafts = restored;
        assert_eq!(drafts.text(context), "x");
    }

    fn draft(operation: u128) -> FailedDraft {
        FailedDraft {
            operation,
            body: Text::new("Retained synthetic text").unwrap(),
            target: Default::default(),
            kind: FailedDraftKind::Send,
            retrying: false,
        }
    }
    #[test]
    fn one_failed_selection_survives_reordering_and_retires_a_removed_operation() {
        use talk_core::ui::{AccountId, AccountScope, ConversationId, HostId, RuntimeGeneration};
        let account = AccountId::from_local(1);
        let context = ConversationRef {
            account: AccountScope {
                account,
                runtime: RuntimeGeneration::from_local(1),
            },
            conversation: ConversationId {
                account,
                host: HostId::from_local(1),
                local: 1,
            },
        };
        let drafts = [draft(3), draft(1), draft(2)];
        assert_eq!(
            selected(&drafts, Some((context, 1)), context)
                .unwrap()
                .1
                .operation,
            1
        );
        assert_eq!(
            selected(&drafts, Some((context, 9)), context)
                .unwrap()
                .1
                .operation,
            3
        );
        let mut other = context;
        other.account.runtime = RuntimeGeneration::from_local(2);
        assert_eq!(
            selected(&drafts, Some((other, 1)), context)
                .unwrap()
                .1
                .operation,
            3
        );
        assert_eq!(next(&drafts, 2, true), Some(3));
        assert_eq!(next(&drafts, 3, false), Some(2));
        assert_eq!(next(&drafts, 9, true), None);
        assert_eq!(next(&[], 1, false), None);
        assert!(selected(&[], None, context).is_none());
    }
}
