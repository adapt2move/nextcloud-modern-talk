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
            && draft.kind != FailedDraftKind::File
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
                FailedDraftKind::File | FailedDraftKind::Send => self
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
                FailedDraftKind::Schedule(_) | FailedDraftKind::EditScheduled(_) => {
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
                    self.message_ui.scheduled_edit = None;
                    self.open_message_deadline(None, None, window, cx);
                    if let Some(local) = self.settings_port.local_datetime(timestamp) {
                        self.message_ui.picker =
                            talk_core::preferences::time::calendar::Calendar::parse(local.as_str());
                    }
                } else {
                    self.composer.focus_handle(cx).focus(window);
                }
            }
            _ => {}
        }
        cx.notify();
    }

    /// A failed text stays honest and quiet: the retained words with a small red
    /// marker and plain-text actions, aligned with message text. Nothing is
    /// replayed by itself; each action remains an explicit typed intent.
    pub(in crate::presentation) fn failed_draft_card(
        &self,
        cx: &Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let context = self.snapshot.selected?;
        let drafts = self.current_failed_drafts(context)?;
        let (index, draft) = selected(drafts, self.message_ui.failed_selection, context)?;
        let operation = draft.operation;
        let t = self.theme;
        let action = |action| LocalAction::MessageIn(context, super::Action::Recovery(action));
        let copy_allowed = !self.background_inert
            && !self.projection_blocked
            && !self.update_restart_pending
            && self
                .selected()
                .is_some_and(|room| room.draft_clipboard == Decision::Allowed);
        let restore = self.can_restore_failed_draft(context, draft, cx);
        let retry = self.can_retry_failed_draft(draft);
        let (label, discard) = failed_labels(draft.kind);
        // A 32 pt target on an 18 pt line: the extra height overlaps its padding.
        let link =
            |id: gpui::ElementId, text: &str, enabled: Availability, color: gpui::Hsla, act| {
                self.button(id, text.to_owned(), ButtonStyle::Quiet, enabled, act, cx)
                    .min_h(px(32.))
                    .my(px(-7.))
                    .px(px(6.))
                    .py_0()
                    .text_size(self.sp(12.))
                    .line_height(self.sp(18.))
                    .text_color(color)
            };
                let separator = || div().text_color(t.muted).child("·");
        let role = if draft.retrying {
            crate::presentation::controls::BannerRole::Info
        } else {
            crate::presentation::controls::BannerRole::Error
        };
        let status_color = role.foreground(t);
        Some(
            self.banner_frame(role)
                .items_start()
                .gap_3()
                .child(
                    div()
                        .w(px(36.))
                        .flex_none()
                        .flex()
                        .justify_center()
                        .pt(px(3.))
                        .child(self.scaled_icon(
                            if draft.retrying { "clock" } else { "info" },
                            16.,
                            status_color,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .max_h(self.sp(44.))
                                .overflow_hidden()
                                .text_size(self.sp(14.))
                                .line_height(self.sp(22.))
                                .opacity(0.72)
                                .child(crate::presentation::selectable::SelectableText::new(
                                    scoped_id("failed-text", context, operation, 0),
                                    draft.body.as_str(),
                                    self.text_selection.clone(),
                                    t,
                                    copy_allowed,
                                )),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .items_center()
                                .gap_x(px(2.))
                                .text_size(self.sp(12.))
                                .line_height(self.sp(18.))
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .mr(px(4.))
                                        .text_color(status_color)
                                        .child(self.l(if draft.retrying {
                                            "Wird erneut versucht …"
                                        } else {
                                            label
                                        })
                                        .to_owned()),
                                )
                                .when(!draft.retrying, |row| {
                                    row.child(separator())
                                        .child(link(
                                            scoped_id("failed-retry", context, operation, 0),
                                            "Erneut versuchen",
                                            Availability::when(
                                                retry,
                                                "Diese Aktion kann gerade nicht erneut versucht werden.",
                                            ),
                                            if retry { t.text } else { t.disabled_fg },
                                            action(RecoveryAction::Retry(operation)),
                                        ))
                                        .when(draft.kind != FailedDraftKind::File, |row| {
                                            row.child(separator()).child(link(
                                                scoped_id("failed-restore", context, operation, 0),
                                                "Bearbeiten",
                                                Availability::when(
                                                    restore,
                                                    "Beende zuerst die aktuelle Eingabe. Ein vorhandener Entwurf wird nicht überschrieben.",
                                                ),
                                                if restore { t.text } else { t.disabled_fg },
                                                action(RecoveryAction::Restore(operation)),
                                            ))
                                        })
                                        .child(separator())
                                        .child(link(
                                            scoped_id("failed-discard", context, operation, 0),
                                            discard,
                                            Availability::when(
                                                !draft.retrying,
                                                "Der Entwurf wird gerade verarbeitet.",
                                            ),
                                            t.danger,
                                            action(RecoveryAction::Discard(operation)),
                                        ))
                                })
                                .when(drafts.len() > 1, |row| {
                                    row.child(
                                        div()
                                            .ml_auto()
                                            .flex()
                                            .items_center()
                                            .child(
                                                self.icon_button(
                                                    scoped_id("failed-previous", context, 0, 0),
                                                    ("arrow-left", "Vorheriger Entwurf"),
                                                    ButtonStyle::Quiet,
                                                    Availability::available(),
                                                    action(RecoveryAction::Previous(operation)),
                                                    cx,
                                                )
                                                .my(px(-7.)),
                                            )
                                            .child(
                                                div()
                                                    .font_family("Google Sans Code")
                                                    .text_size(self.sp(12.))
                                                    .child(format!("{}/{}", index + 1, drafts.len())),
                                            )
                                            .child(
                                                self.icon_button(
                                                    scoped_id("failed-next", context, 0, 0),
                                                    ("arrow-right", "Nächster Entwurf"),
                                                    ButtonStyle::Quiet,
                                                    Availability::available(),
                                                    action(RecoveryAction::Next(operation)),
                                                    cx,
                                                )
                                                .my(px(-7.)),
                                            ),
                                    )
                                }),
                        ),
                )
                .into_any_element(),
        )
    }
}

/// Status wording and the destructive verb for each kind of retained failure.
/// Only a typed send/file is deleted; an edit or schedule never reads as
/// deleting the message it belongs to.
fn failed_labels(kind: FailedDraftKind) -> (&'static str, &'static str) {
    match kind {
        FailedDraftKind::File | FailedDraftKind::Send => ("Nicht gesendet", "Löschen"),
        FailedDraftKind::Edit(_) => ("Änderung nicht gespeichert", "Verwerfen"),
        FailedDraftKind::Schedule(_) | FailedDraftKind::EditScheduled(_) => {
            ("Planung nicht gespeichert", "Verwerfen")
        }
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
    fn only_an_unsent_text_or_file_is_deleted_and_edits_are_never_called_deletion() {
        assert_eq!(
            failed_labels(FailedDraftKind::Send),
            ("Nicht gesendet", "Löschen")
        );
        assert_eq!(
            failed_labels(FailedDraftKind::File),
            ("Nicht gesendet", "Löschen")
        );
        for kind in [
            FailedDraftKind::Edit(1),
            FailedDraftKind::Schedule(5),
            FailedDraftKind::EditScheduled(2),
        ] {
            assert_eq!(failed_labels(kind).1, "Verwerfen");
            assert_ne!(failed_labels(kind).0, "Nicht gesendet");
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
