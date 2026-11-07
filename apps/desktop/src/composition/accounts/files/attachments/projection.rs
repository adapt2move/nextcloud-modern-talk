//! Pending rows are overlays, never canonical history or server permissions.
use super::*;
use talk_core::ui::presentation::{DeliveryState, MessageView};

impl Accounts {
    pub(in crate::composition::accounts) fn project_uploads(&mut self) {
        let context = self.attachment_owner.batch.as_ref().map(|b| b.context);
        if let Some(context) = context.filter(|c| self.snapshot.selected == Some(*c)) {
            self.reconcile_uploads(context);
        }
        let uploads = page(
            self.attachment_owner
                .batch
                .as_ref()
                .filter(|b| Some(b.context) == self.snapshot.selected)
                .into_iter()
                .flat_map(|b| b.files.iter().map(|f| f.view.clone())),
        );
        let running = self
            .attachment_owner
            .batch
            .as_ref()
            .is_some_and(|b| Some(b.context) == self.snapshot.selected && b.running);
        let mut changed = false;
        if let Some(panel) = self.snapshot.message_panel.as_mut() {
            changed = panel.uploads != uploads || panel.uploads_running != running;
            panel.uploads = uploads;
            panel.uploads_running = running;
        }
        let mut rows: Vec<_> = self
            .snapshot
            .messages
            .as_slice()
            .iter()
            .filter(|m| !UploadView::is_message_id(m.local_id))
            .cloned()
            .collect();
        if let Some(batch) = self.attachment_owner.batch.as_ref().filter(|b| {
            Some(b.context) == self.snapshot.selected
                && self.snapshot.content != ContentState::Denied
        }) {
            let author = self
                .snapshot
                .accounts
                .as_slice()
                .iter()
                .find(|a| a.scope == batch.context.account)
                .map_or("", |a| a.self_name.as_str());
            for file in &batch.files {
                let Some(outgoing) = file.outgoing.as_ref().filter(|o| {
                    file.view.server_message.is_none()
                        && o.operation.is_none_or(|op| !self.outbox.contains(op))
                }) else {
                    continue;
                };
                let mut row = MessageView {
                    local_id: file.view.message_id(),
                    author: text(author),
                    initials: initials(author),
                    time_label: text(&messaging::time_label(outgoing.created)),
                    body: outgoing.caption.clone(),
                    own: true,
                    delivery: match file.view.state {
                        UploadState::Uncertain => DeliveryState::Uncertain,
                        UploadState::Failed(_) => DeliveryState::Failed,
                        _ => DeliveryState::Pending,
                    },
                    attachments: Page::empty(),
                    reply_count: 0,
                    metadata: Default::default(),
                };
                row.metadata.local_operation = outgoing.operation.map(OperationId::as_local);
                row.metadata.local_order = Some(outgoing.request.0);
                row.metadata.timestamp = outgoing.created;
                rows.push(row);
            }
        }
        // Canonical rows retain server order. Pending text/files share the UI
        // admission order; file rows never gain a fabricated server ID.
        rows.sort_by_key(|m| {
            (
                m.metadata.local_order.is_some(),
                m.metadata.local_order.unwrap_or(0),
            )
        });
        while rows.len() > 100 {
            let index = rows
                .iter()
                .position(|m| m.metadata.local_order.is_none())
                .unwrap_or(0);
            rows.remove(index);
        }
        let messages = page(rows);
        changed |= self.snapshot.messages != messages;
        self.snapshot.messages = messages;
        if changed {
            self.touch();
        }
    }

    fn reconcile_uploads(&mut self, context: ConversationRef) {
        let Some(batch) = self.attachment_owner.batch.as_ref() else {
            return;
        };
        let seen: Vec<_> = batch
            .files
            .iter()
            .filter_map(|file| {
                let operation = file.outgoing.as_ref()?.operation?;
                let message = self
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .chain(
                        self.history
                            .get(context)
                            .into_iter()
                            .flat_map(|h| h.messages.as_slice()),
                    )
                    .find(|m| {
                        m.own
                            && m.metadata.local_operation == Some(operation.as_local())
                            && matches!(
                                m.delivery,
                                DeliveryState::Received | DeliveryState::ServerAccepted
                            )
                    })?;
                let local = self
                    .snapshot
                    .content_panel
                    .assets
                    .as_slice()
                    .iter()
                    .find(|a| a.target == preview_target(context, file.view.id));
                let preview = local.is_some_and(|a| {
                    matches!(
                        a.state,
                        talk_core::ui::content::AssetState::Loading
                            | talk_core::ui::content::AssetState::Ready { .. }
                    )
                });
                let rich: Vec<_> = message
                    .metadata
                    .rich
                    .as_slice()
                    .iter()
                    .filter(|r| r.kind.as_str() == "file")
                    .collect();
                let remote_terminal = rich.as_slice().first().is_none_or(|rich| {
                    let target = talk_core::ui::content::AssetTarget::Thumbnail(AttachmentRef {
                        context,
                        message: message.local_id,
                        placeholder: rich.placeholder.clone(),
                    });
                    self.snapshot
                        .content_panel
                        .assets
                        .as_slice()
                        .iter()
                        .any(|a| {
                            a.target == target
                                && matches!(
                                    a.state,
                                    talk_core::ui::content::AssetState::Ready { .. }
                                        | talk_core::ui::content::AssetState::Denied
                                        | talk_core::ui::content::AssetState::Missing
                                        | talk_core::ui::content::AssetState::Unsupported
                                )
                        })
                });
                Some((
                    file.view.id,
                    message.local_id,
                    preview && rich.len() == 1 && !message.metadata.deleted && !remote_terminal,
                ))
            })
            .collect();
        let resume_allowed = allowed(&self.snapshot, context);
        let mut confirmed = None;
        let batch = self
            .attachment_owner
            .batch
            .as_mut()
            .expect("observed batch");
        let mut resolved_uncertain = false;
        for file in &mut batch.files {
            let (bytes, total, _) = file.progress.snapshot();
            file.view.transferred = bytes;
            if let Some(total) = total {
                file.view.bytes = Some(total);
            }
            if let Some((_, message, _)) = seen.iter().find(|(id, _, _)| *id == file.view.id) {
                resolved_uncertain |= file.view.state == UploadState::Uncertain;
                file.view.state = UploadState::Shared;
                file.view.server_message = Some(*message);
                if let Some(outgoing) = &file.outgoing {
                    batch
                        .captions
                        .retain(|(request, _)| *request != outgoing.request);
                    confirmed = Some(outgoing.request);
                }
            }
        }
        batch.files.retain(|file| {
            let keep = !seen
                .iter()
                .any(|(id, _, keep)| *id == file.view.id && !keep);
            if !keep {
                file.progress.cancel();
            }
            keep
        });
        if resolved_uncertain
            && resume_allowed
            && !batch
                .files
                .iter()
                .any(|f| f.view.state == UploadState::Uncertain)
            && batch
                .files
                .iter()
                .any(|f| f.view.submitted && f.view.state == UploadState::Selected)
        {
            batch.running = true;
        }
        if let Some(request) = confirmed
            && let Some(panel) = self
                .snapshot
                .message_panel
                .as_mut()
                .filter(|p| p.context == context)
        {
            panel.files_sent = Some(request);
        }
    }
}

pub(super) fn preview_target(
    context: ConversationRef,
    id: u64,
) -> talk_core::ui::content::AssetTarget {
    talk_core::ui::content::AssetTarget::Thumbnail(AttachmentRef {
        context,
        message: (1u128 << 126) | u128::from(id),
        placeholder: text("upload"),
    })
}
