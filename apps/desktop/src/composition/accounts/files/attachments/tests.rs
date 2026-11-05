use super::*;

fn model() -> (Accounts, ConversationRef) {
    let mut model = crate::composition::accounts::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat-content"));
    let context = model.snapshot.selected.unwrap();
    (model, context)
}
fn paths(names: &[&str]) -> Page<Text<16384>, 8> {
    page(names.iter().map(|name| text(&format!("/tmp/{name}"))))
}
fn result(model: &mut Accounts, id: u64, step: Step, result: Result<Reply, ErrorKind>) -> Work {
    let context = model.snapshot.selected.unwrap();
    let operation = OperationId::from_local(100 + u128::from(id) * 10 + step as u128);
    let work = Work {
        context,
        operation,
        file: id,
        step,
    };
    model.attachment_owner.flight = Some(work);
    model.complete_file_work(
        work,
        result
            .map(|result| {
                RuntimeReply::Protocol(ProtocolReply::Messaging {
                    conversation: context.conversation,
                    operation,
                    result,
                })
            })
            .map_err(IdentityError::Request),
    );
    work
}
#[test]
fn staging_deduplicates_without_uploading_and_rejects_whole_overflow_selection() {
    let (mut model, context) = model();
    model
        .stage_files(context, paths(&["one.png", "one.png", "two.json"]))
        .unwrap();
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()
            .len(),
        2
    );
    assert!(model.pending.is_empty());
    assert!(model.attachment_owner.flight.is_none());
    assert!(
        model
            .attachment_owner
            .batch
            .as_ref()
            .unwrap()
            .next()
            .is_none()
    );
    let before = model
        .snapshot
        .message_panel
        .as_ref()
        .unwrap()
        .uploads
        .clone();
    assert_eq!(
        model.stage_files(context, paths(&["3", "4", "5", "6", "7", "8", "9"])),
        Err(SubmitError::Busy)
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.uploads, before);
    assert_eq!(panel.file_error, Some(UploadFailure::TooMany));
    assert_eq!(
        model.stage_files(context, page([text("relative.png")])),
        Err(SubmitError::Unavailable(UnavailableReason::Unsupported))
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().uploads,
        before
    );
}
#[test]
fn read_only_or_obsolete_selection_cannot_stage_or_start_files() {
    let (mut model, context) = model();
    model.snapshot.content = ContentState::Denied;
    assert!(model.stage_files(context, paths(&["one.png"])).is_err());
    assert!(model.attachment_owner.batch.is_none());
    model.snapshot.content = ContentState::Ready;
    model.stage_files(context, paths(&["one.png"])).unwrap();
    for target in [
        ComposeTarget {
            reply_to: Some(1),
            ..Default::default()
        },
        ComposeTarget {
            silent: true,
            ..Default::default()
        },
        ComposeTarget {
            thread: Some(1),
            ..Default::default()
        },
    ] {
        assert_eq!(
            model.send_files(RequestId(1), context, text("caption"), target),
            Err(SubmitError::Unavailable(UnavailableReason::Unsupported))
        );
    }
    assert!(!model.attachment_owner.batch.as_ref().unwrap().running);
    model.snapshot.selected = None;
    assert!(
        model
            .send_files(
                RequestId(2),
                context,
                text("caption"),
                ComposeTarget::default()
            )
            .is_err()
    );
    assert!(model.pending.is_empty());
}
#[test]
fn sequential_batch_shares_once_and_retries_sharing_without_reuploading_or_repeating_caption() {
    let (mut model, context) = model();
    model
        .stage_files(context, paths(&["one.png", "two.json"]))
        .unwrap();
    model
        .send_files(
            RequestId(10),
            context,
            text("caption"),
            ComposeTarget::default(),
        )
        .unwrap();
    assert_eq!(
        model.send_files(
            RequestId(11),
            context,
            text("duplicate"),
            ComposeTarget::default()
        ),
        Err(SubmitError::Busy)
    );
    assert!(matches!(
        model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((1, Step::Upload, Command::UploadFile { .. }))
    ));
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/one"),
            bytes: 100,
        }),
    );
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(), Some((1, Step::Share, Command::ShareFile { path, caption })) if path.as_str()=="/draft/one" && caption.as_str()=="caption")
    );
    result(&mut model, 1, Step::Share, Ok(Reply::Changed));
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().files_sent,
        Some(RequestId(10))
    );
    result(
        &mut model,
        2,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/two"),
            bytes: 200,
        }),
    );
    result(&mut model, 2, Step::Share, Err(ErrorKind::PermissionDenied));
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()[0]
            .state,
        UploadState::Failed(UploadFailure::Denied)
    );
    model.retry_file(context, 2).unwrap();
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(), Some((2, Step::Share, Command::ShareFile { path, caption })) if path.as_str()=="/draft/two" && caption.as_str().is_empty())
    );
    result(&mut model, 2, Step::Share, Ok(Reply::Changed));
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert!(panel.uploads.as_slice().is_empty());
    assert!(!panel.uploads_running);
    assert_eq!(panel.files_sent, Some(RequestId(10)));
}
#[test]
fn uncertain_write_blocks_replay_and_preserves_other_files_and_caption() {
    for error in [
        ErrorKind::Uncertain,
        ErrorKind::Transport,
        ErrorKind::Deadline,
        ErrorKind::Malformed,
    ] {
        let (mut model, context) = model();
        model
            .stage_files(context, paths(&["one.png", "two.png"]))
            .unwrap();
        model
            .send_files(
                RequestId(10),
                context,
                text("preserved"),
                ComposeTarget::default(),
            )
            .unwrap();
        result(&mut model, 1, Step::Upload, Err(error));
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .uploads
                .as_slice()[0]
                .state,
            UploadState::Uncertain
        );
        assert!(!model.attachment_owner.batch.as_ref().unwrap().running);
        assert_eq!(model.retry_file(context, 1), Err(SubmitError::Busy));
        assert_eq!(
            model.send_files(
                RequestId(11),
                context,
                text("retry"),
                ComposeTarget::default()
            ),
            Err(SubmitError::Busy)
        );
        assert!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .files_sent
                .is_none()
        );
        assert_eq!(
            model
                .attachment_owner
                .batch
                .as_ref()
                .unwrap()
                .caption
                .as_ref()
                .unwrap()
                .1
                .as_str(),
            "preserved"
        );
        model.remove_file(context, 1).unwrap();
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .uploads
                .as_slice()
                .len(),
            1
        );
    }
}
#[test]
fn ordinary_upload_failure_does_not_discard_the_rest_of_the_batch() {
    let (mut model, context) = model();
    model
        .stage_files(context, paths(&["missing.png", "small.png"]))
        .unwrap();
    model
        .send_files(
            RequestId(1),
            context,
            text("caption"),
            ComposeTarget::default(),
        )
        .unwrap();
    result(&mut model, 1, Step::Upload, Err(ErrorKind::NotFound));
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()[0]
            .state,
        UploadState::Failed(UploadFailure::Missing)
    );
    assert!(matches!(
        model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((2, Step::Upload, _))
    ));
    assert!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .files_sent
            .is_none()
    );
}
#[test]
fn cancelled_scope_or_expired_batch_cannot_resume_from_a_late_upload_result() {
    let (mut model, context) = model();
    model.stage_files(context, paths(&["old.png"])).unwrap();
    let work = Work {
        context,
        operation: OperationId::from_local(77),
        file: 1,
        step: Step::Upload,
    };
    model.attachment_owner.flight = Some(work);
    model.clear_attachment_work();
    model.stage_files(context, paths(&["new.png"])).unwrap();
    model.complete_file_work(
        work,
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation: work.operation,
            result: Reply::Uploaded {
                path: text("/old/upload"),
                bytes: 4,
            },
        })),
    );
    let batch = model.attachment_owner.batch.as_ref().unwrap();
    assert_eq!(batch.files.len(), 1);
    assert_eq!(batch.files[0].view.name.as_str(), "new.png");
    assert!(batch.files[0].remote.is_none());
    model.attachment_owner.batch.as_mut().unwrap().created =
        Instant::now() - Duration::from_secs(31 * 60);
    model.tick_file_work();
    assert!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()
            .is_empty()
    );
    assert!(model.attachment_owner.batch.is_none());
}
#[test]
fn idle_file_owner_does_not_trigger_a_render_tick() {
    let (mut model, _) = model();
    model.dirty = false;
    model.tick_file_work();
    assert!(!model.dirty);
}
fn export(model: &mut Accounts, request: u64) -> (Work, protocol::content::TransferProgress) {
    let context = model.snapshot.selected.unwrap();
    let reference = AttachmentRef {
        context,
        message: 101,
        placeholder: text("photo"),
    };
    let work = Work {
        context,
        operation: OperationId::from_local(u128::from(123 + request)),
        file: request,
        step: Step::Export,
    };
    let progress = protocol::content::TransferProgress::default();
    model.attachment_owner.flight = Some(work);
    model.attachment_owner.export = Some(Export {
        work,
        saved: None,
        progress: progress.clone(),
        deadline: Instant::now() + Duration::from_secs(120),
    });
    model.content_owner.transfer = Some(progress.clone());
    model.snapshot.content_panel.transfer = Some(DownloadView {
        request: RequestId(request),
        attachment: reference,
        name: text("photo.png"),
        received: 0,
        total: Some(4),
        state: DownloadState::Transferring,
    });
    model.project_file_export();
    (work, progress)
}
#[test]
fn file_promise_success_requires_actual_publication_not_just_an_acknowledgement() {
    for published in [false, true] {
        let (mut model, context) = model();
        let (work, progress) = export(&mut model, 55);
        if published {
            progress.published();
        }
        model.complete_file_work(
            work,
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation: work.context.conversation,
                operation: work.operation,
                result: Reply::FileSaved { bytes: 4 },
            })),
        );
        assert_eq!(model.snapshot.file_export.unwrap().saved, Some(published));
        assert!(model.attachment_owner.export.is_some());
        assert!(model.content_owner.transfer.is_none());
        // Retiring the former account cannot reverse an observed publication.
        model.clear_attachment_work();
        model.snapshot.selected = None;
        model.snapshot.active_account = None;
        assert_eq!(model.snapshot.file_export.unwrap().saved, Some(published));
        model.finish_file_export(RequestId(55), context);
        assert!(model.snapshot.file_export.is_none());
        assert!(model.attachment_owner.export.is_none());
    }
}
#[test]
fn export_timeout_or_scope_retirement_cancels_pending_writes_and_retains_the_outcome() {
    let (mut model, context) = model();
    let (work, progress) = export(&mut model, 55);
    model.attachment_owner.export.as_mut().unwrap().deadline = Instant::now();
    model.tick_file_work();
    assert!(progress.is_cancelled());
    assert_eq!(model.snapshot.file_export.unwrap().saved, Some(false));
    model.clear_attachment_work();
    model.complete_file_work(
        work,
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation: work.operation,
            result: Reply::FileSaved { bytes: 4 },
        })),
    );
    assert_eq!(model.snapshot.file_export.unwrap().saved, Some(false));
    model.finish_file_export(RequestId(55), context);
    let (_, progress) = export(&mut model, 56);
    model.clear_attachment_work();
    assert!(progress.is_cancelled());
    assert_eq!(model.snapshot.file_export.unwrap().saved, Some(false));
    // A late acknowledgement cannot release a newer native promise's slot.
    model.finish_file_export(RequestId(55), context);
    assert_eq!(model.snapshot.file_export.unwrap().request, RequestId(56));
    model.attachment_owner.export.as_mut().unwrap().deadline =
        Instant::now() - Duration::from_secs(6);
    model.tick_file_work();
    assert!(model.snapshot.file_export.is_none());
    assert!(model.attachment_owner.export.is_none());
}
#[test]
fn explicit_cancel_finishes_the_native_promise_without_waiting_for_a_network_reply() {
    for published in [false, true] {
        let (mut model, context) = model();
        let (work, progress) = export(&mut model, 55);
        if published {
            progress.published();
        }
        model.cancel_download();
        assert!(progress.is_cancelled());
        assert_eq!(progress.is_published(), published);
        assert!(model.attachment_owner.flight.is_none());
        assert!(model.content_owner.transfer.is_none());
        assert_eq!(model.snapshot.file_export.unwrap().saved, Some(published));
        assert_eq!(
            model
                .snapshot
                .content_panel
                .transfer
                .as_ref()
                .unwrap()
                .state,
            if published {
                DownloadState::Saved
            } else {
                DownloadState::Cancelled
            }
        );
        model.complete_file_work(
            work,
            Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
                conversation: context.conversation,
                operation: work.operation,
                result: Reply::FileSaved { bytes: 4 },
            })),
        );
        assert_eq!(model.snapshot.file_export.unwrap().saved, Some(published));
        model.finish_file_export(RequestId(55), context);
        assert!(model.snapshot.file_export.is_none());
    }
}
#[test]
fn stale_drag_source_or_account_cannot_start_an_export_and_tombstones_dominate_pin_copies() {
    let (mut model, context) = model();
    let reference = AttachmentRef {
        context,
        message: 101,
        placeholder: text("photo"),
    };
    let source = file_source(&model.snapshot, &reference).unwrap().clone();
    let mut replacement = source.clone();
    replacement.id = Some(text("replacement"));
    assert_eq!(
        model.export_file(
            RequestId(1),
            context,
            reference.clone(),
            &replacement,
            text("/tmp/out.png")
        ),
        Err(SubmitError::StaleScope)
    );
    assert!(model.snapshot.file_export.is_none());
    assert!(model.pending.is_empty());
    let previous = model.snapshot.messages.as_slice()[0].clone();
    model
        .snapshot
        .message_panel
        .as_mut()
        .unwrap()
        .pinned_messages = page([previous]);
    model.snapshot.messages = page(model.snapshot.messages.as_slice().iter().cloned().map(
        |mut message| {
            if message.local_id == 101 {
                message.metadata.deleted = true;
            }
            message
        },
    ));
    assert!(file_source(&model.snapshot, &reference).is_none());
    assert!(
        model
            .export_file(
                RequestId(2),
                context,
                reference,
                &source,
                text("/tmp/out.png")
            )
            .is_err()
    );
    assert!(model.snapshot.file_export.is_none());
}
