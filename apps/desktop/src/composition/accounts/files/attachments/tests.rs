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
#[test]
fn selected_cloud_file_is_scoped_and_enters_the_share_queue_without_upload() {
    let (mut model, context) = model();
    model.snapshot.content_panel.file_browser = Some(talk_core::ui::content::FileBrowserView {
        account: context.account,
        path: text("/Plans"),
        section: talk_core::ui::content::FileSection::All,
        sort: talk_core::ui::content::FileSort::Name,
        generation: 1,
        pagination: None,
        search_available: true,
        favorites_available: true,
        has_listing: true,
        entries: page([talk_core::ui::content::CloudFileView {
            name: text("plan.pdf"),
            path: text("/Plans/plan.pdf"),
            directory: false,
            size: Some(42),
            modified: None,
            modified_unix: None,
            mime: None,
            file_id: None,
        }]),
        loading: false,
        offline: false,
        error: None,
    });
    assert_eq!(
        model.stage_cloud_file(context, text("/old/plan.pdf")),
        Err(SubmitError::StaleScope)
    );
    model
        .stage_cloud_file(context, text("/Plans/plan.pdf"))
        .unwrap();
    model
        .stage_cloud_file(context, text("/Plans/plan.pdf"))
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
        1
    );
    model
        .send_files(
            RequestId(12),
            context,
            text("hello"),
            ComposeTarget::default(),
        )
        .unwrap();
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((_, Step::Share, Command::ShareExistingFile { path, caption, .. }))
        if path.as_str() == "/Plans/plan.pdf" && caption.as_str() == "hello")
    );
}

#[test]
fn cloud_multi_selection_rejects_stale_entries_without_partial_staging() {
    let (mut model, context) = model();
    model.snapshot.content_panel.file_browser = Some(talk_core::ui::content::FileBrowserView {
        account: context.account,
        path: text("/"),
        section: talk_core::ui::content::FileSection::All,
        sort: talk_core::ui::content::FileSort::Name,
        generation: 1,
        pagination: None,
        search_available: true,
        favorites_available: true,
        has_listing: true,
        entries: page([talk_core::ui::content::CloudFileView {
            name: text("one"),
            path: text("/one"),
            directory: false,
            size: None,
            modified: None,
            modified_unix: None,
            mime: None,
            file_id: None,
        }]),
        loading: false,
        offline: false,
        error: None,
    });
    assert_eq!(
        model.stage_cloud_files(context, page([text("/one"), text("/gone")])),
        Err(SubmitError::StaleScope)
    );
    assert!(model.attachment_owner.batch.is_none());
    model
        .stage_cloud_files(context, page([text("/one"), text("/one")]))
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
        1
    );
}
#[test]
fn attachment_metadata_is_preserved_for_each_queued_share() {
    let (mut model, context) = model();
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    panel.media_caption = true;
    let allowed = talk_core::ui::conversation::Availability {
        implemented: true,
        server: talk_core::ui::presentation::FeatureSupport::Supported,
        allowed: Decision::Allowed,
    };
    let mut features = panel.features.as_slice().to_vec();
    features
        .retain(|(feature, _)| !matches!(feature, MessageFeature::Reply | MessageFeature::Threads));
    features.push((MessageFeature::Reply, allowed));
    features.push((MessageFeature::Threads, allowed));
    panel.features = page(features);
    model.stage_files(context, paths(&["one.png"])).unwrap();
    let target = ComposeTarget {
        reply_to: Some(7),
        thread: Some(8),
        silent: true,
    };
    model
        .send_files(RequestId(31), context, text("caption"), target)
        .unwrap();
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/one"),
            bytes: 4,
        }),
    );
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((1, Step::Share, Command::ShareFile { options, .. }))
        if options.reply_to.is_some_and(|message| message.sequence == 7)
        && options.thread.is_some_and(|message| message.sequence == 8) && options.silent)
    );
}
#[test]
fn clipboard_image_stays_bounded_and_uses_byte_upload() {
    use talk_core::ui::ClipboardImage;
    assert!(ClipboardImage::new(&[]).is_none());
    assert!(ClipboardImage::new(&vec![0; 4 * 1024 * 1024 + 1]).is_none());
    let (mut model, context) = model();
    let image = ClipboardImage::new(&[1, 2, 3]).unwrap();
    model
        .stage_clipboard_image(context, text("clipboard.png"), image)
        .unwrap();
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()[0]
            .bytes,
        Some(3)
    );
    model
        .send_files(RequestId(32), context, text(""), ComposeTarget::default())
        .unwrap();
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((_, Step::Upload, Command::UploadImage { image, .. })) if image.as_slice() == [1, 2, 3])
    );
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
    if step == Step::Share
        && let Some(file) = model
            .attachment_owner
            .batch
            .as_mut()
            .and_then(|b| b.files.iter_mut().find(|f| f.view.id == id))
        && let Some(outgoing) = file.outgoing.as_mut()
    {
        outgoing.operation = Some(operation);
    }
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
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    panel.media_caption = false;
    panel.features = page(
        panel
            .features
            .as_slice()
            .iter()
            .filter(|(feature, _)| {
                !matches!(feature, MessageFeature::Reply | MessageFeature::Threads)
            })
            .copied(),
    );
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
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(), Some((1, Step::Share, Command::ShareFile { path, caption, .. })) if path.as_str()=="/draft/one" && caption.as_str()=="caption")
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
            .as_slice()[1]
            .state,
        UploadState::Failed(UploadFailure::Denied)
    );
    model.retry_file(context, 2).unwrap();
    assert!(
        matches!(model.attachment_owner.batch.as_ref().unwrap().next(), Some((2, Step::Share, Command::ShareFile { path, caption, .. })) if path.as_str()=="/draft/two" && caption.as_str().is_empty())
    );
    result(&mut model, 2, Step::Share, Ok(Reply::Changed));
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert!(
        panel
            .uploads
            .as_slice()
            .iter()
            .all(|f| f.state == UploadState::Confirming)
    );
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
        result(
            &mut model,
            1,
            Step::Upload,
            Ok(Reply::Uploaded {
                path: text("/draft/one"),
                bytes: 100,
            }),
        );
        result(&mut model, 1, Step::Share, Err(error));
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
            model.attachment_owner.batch.as_ref().unwrap().captions[0]
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

#[test]
fn admission_projects_a_chat_row_and_progress_without_waiting_for_network() {
    let (mut model, context) = model();
    model
        .stage_files(context, paths(&["one.png", "two.json"]))
        .unwrap();
    let before = model.snapshot.messages.as_slice().len();
    model
        .send_files(
            RequestId(10),
            context,
            text("My favourite"),
            ComposeTarget::default(),
        )
        .unwrap();
    let rows = model.snapshot.messages.as_slice();
    assert_eq!(rows.len(), before + 2);
    assert_eq!(rows[before].body.as_str(), "My favourite");
    assert!(rows[before + 1].body.as_str().is_empty());
    assert!(
        rows[before..]
            .iter()
            .all(|row| row.own && UploadView::is_message_id(row.local_id))
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().files_queued,
        Some(RequestId(10))
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().files_sent,
        None
    );
    let batch = model.attachment_owner.batch.as_mut().unwrap();
    batch.files[0].view.state = UploadState::Uploading;
    batch.files[0].progress.update(64, Some(100));
    model.project_uploads();
    let file = &model
        .snapshot
        .message_panel
        .as_ref()
        .unwrap()
        .uploads
        .as_slice()[0];
    assert_eq!((file.transferred, file.bytes), (64, Some(100)));
    assert!(model.pending.is_empty());
}

#[test]
fn late_canonical_message_settles_an_uncertain_share_once_and_preserves_other_files() {
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
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/one"),
            bytes: 100,
        }),
    );
    let work = result(&mut model, 1, Step::Share, Err(ErrorKind::Uncertain));
    let mut canonical = model.snapshot.messages.as_slice()[0].clone();
    canonical.local_id = 777;
    canonical.own = true;
    canonical.delivery = talk_core::ui::presentation::DeliveryState::Received;
    canonical.metadata.local_operation = Some(work.operation.as_local());
    canonical.metadata.rich = Page::empty();
    for correct in [false, true] {
        canonical.own = correct;
        model.snapshot.messages.try_push(canonical.clone()).unwrap();
        model.project_uploads();
        if !correct {
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
            model.snapshot.messages = page(
                model
                    .snapshot
                    .messages
                    .as_slice()
                    .iter()
                    .filter(|m| m.local_id != 777)
                    .cloned(),
            );
        }
    }
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.uploads.as_slice().len(), 1);
    assert_eq!(panel.uploads.as_slice()[0].id, 2);
    assert!(panel.uploads_running);
    assert_eq!(panel.files_sent, Some(RequestId(10)));
    assert!(
        !model
            .snapshot
            .messages
            .as_slice()
            .iter()
            .any(|m| m.local_id == (1u128 << 126) | 1)
    );
    assert!(matches!(
        model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((2, Step::Upload, _))
    ));
    model.complete_file_work(work, Err(IdentityError::Request(ErrorKind::Deadline)));
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

#[test]
fn a_cancelled_or_replaced_local_preview_never_becomes_ready() {
    let (mut model, context) = model();
    model.stage_files(context, paths(&["one.png"])).unwrap();
    let job = model.take_content_image().unwrap();
    assert!(job.local.is_some());
    model.remove_file(context, 1).unwrap();
    model.stage_files(context, paths(&["one.png"])).unwrap();
    assert!(!model.finish_content_image(&job, Some(128)));
    let next = model.take_content_image().unwrap();
    assert_ne!(job.key, next.key);
    assert!(model.finish_content_image(&next, Some(128)));
    model.snapshot.selected = None;
    assert!(!model.finish_content_image(&next, Some(128)));
}

#[test]
fn retrying_an_unshared_upload_requires_an_explicit_action_and_does_not_submit_new_drafts() {
    let (mut model, context) = model();
    model.stage_files(context, paths(&["one.json"])).unwrap();
    model
        .send_files(
            RequestId(10),
            context,
            text("caption"),
            ComposeTarget::default(),
        )
        .unwrap();
    result(&mut model, 1, Step::Upload, Err(ErrorKind::Uncertain));
    assert!(
        model
            .attachment_owner
            .batch
            .as_ref()
            .unwrap()
            .next()
            .is_none()
    );
    model.stage_files(context, paths(&["new.json"])).unwrap();
    model.retry_file(context, 1).unwrap();
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/one"),
            bytes: 10,
        }),
    );
    result(&mut model, 1, Step::Share, Ok(Reply::Changed));
    assert!(
        model
            .attachment_owner
            .batch
            .as_ref()
            .unwrap()
            .next()
            .is_none()
    );
    assert!(
        !model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .uploads
            .as_slice()[1]
            .submitted
    );
}

#[test]
fn correlated_image_keeps_local_pixels_until_the_remote_preview_or_denial_arrives() {
    use talk_core::ui::content::{AssetState, AssetTarget, AssetView};
    let (mut model, context) = model();
    model.stage_files(context, paths(&["one.png"])).unwrap();
    let job = model.take_content_image().unwrap();
    assert!(model.finish_content_image(&job, Some(128)));
    model
        .send_files(
            RequestId(10),
            context,
            text("caption"),
            ComposeTarget::default(),
        )
        .unwrap();
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/one"),
            bytes: 100,
        }),
    );
    let work = result(&mut model, 1, Step::Share, Ok(Reply::Changed));
    let mut canonical = model.snapshot.messages.as_slice()[0].clone();
    canonical.local_id = 777;
    canonical.own = true;
    canonical.delivery = talk_core::ui::presentation::DeliveryState::Received;
    canonical.metadata.local_operation = Some(work.operation.as_local());
    assert_eq!(
        canonical
            .metadata
            .rich
            .as_slice()
            .iter()
            .filter(|o| o.kind.as_str() == "file")
            .count(),
        1
    );
    model.snapshot.messages.try_push(canonical.clone()).unwrap();
    model.project_uploads();
    let file = &model
        .snapshot
        .message_panel
        .as_ref()
        .unwrap()
        .uploads
        .as_slice()[0];
    assert_eq!(
        (file.state, file.server_message),
        (UploadState::Shared, Some(777))
    );
    assert!(
        !model
            .snapshot
            .messages
            .as_slice()
            .iter()
            .any(|m| m.local_id == file.message_id())
    );
    assert!(model.asset_is_current(&job.target));
    model
        .stage_files(context, paths(&["another.json"]))
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
    assert!(model.asset_is_current(&job.target));
    model.set_asset(AssetView {
        target: AssetTarget::Thumbnail(AttachmentRef {
            context,
            message: 777,
            placeholder: canonical.metadata.rich.as_slice()[0].placeholder.clone(),
        }),
        state: AssetState::Denied,
        emoji: None,
        color: None,
        document_pages: None,
    });
    model.project_uploads();
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
    assert!(!model.asset_is_current(&job.target));
    assert!(!model.finish_content_image(&job, Some(128)));
}

#[test]
fn dnd_suppresses_notifications_without_retiring_the_voice_attachment_owner() {
    let (mut model, context) = model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat-voice"));
    assert!(model.voice_allowed(context));
    model.status_pending.push((
        context.account,
        talk_core::preferences::RequestId(41),
        talk_core::preferences::account::StatusChoice::DoNotDisturb,
    ));
    assert!(model.suppress_account_notifications(context.account));
    assert!(model.voice_allowed(context));
    model
        .send_voice_bytes(
            RequestId(42),
            context,
            protocol::content::VoiceBytes::new(vec![1, 2, 3, 4]).unwrap(),
        )
        .unwrap();
    assert!(matches!(
        model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((_, Step::Upload, Command::UploadVoice { .. }))
    ));
    assert_eq!(
        model.stage_files(context, paths(&["other.txt"])),
        Err(SubmitError::Busy)
    );
    result(
        &mut model,
        1,
        Step::Upload,
        Ok(Reply::Uploaded {
            path: text("/draft/voice"),
            bytes: 4,
        }),
    );
    assert!(matches!(
        model.attachment_owner.batch.as_ref().unwrap().next(),
        Some((_, Step::Share, Command::ShareFile { voice_message: true, caption, options, .. }))
            if caption.as_str().is_empty() && options == Default::default()
    ));
}
