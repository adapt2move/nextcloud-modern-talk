//! No GPUI window or network worker: exercise the real cache/content reducers
//! with authored image descriptors and bounded PNG bytes.
use super::*;
use crate::composition::accounts::content::ImageJob;
use image::ImageEncoder;
use talk_core::protocol::content::ImageBytes;
use talk_core::ui::content::{AssetState, AssetTarget, AssetView, AttachmentRef};

fn png() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(&[20, 40, 80, 255], 1, 1, image::ExtendedColorType::Rgba8)
        .unwrap();
    bytes
}
fn prepare(local: bool) -> (Accounts, ConversationRef) {
    let mut model = super::super::super::tests::model();
    model.snapshot = super::super::super::super::fixtures::snapshot(Some("chat-content"));
    let mut message = model.snapshot.messages.as_slice()[0].clone();
    message.local_id = 7;
    let scope = model.snapshot.active_account.unwrap();
    install(&mut model, scope);
    let context = if local {
        model.restore_cache_views();
        model.snapshot.conversations.as_slice()[0].context
    } else {
        let room = observed(scope);
        let context = ConversationRef {
            account: scope,
            conversation: room.id,
        };
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room.clone()]),
            },
        );
        model.snapshot.conversations = page([conversation(&room, scope)]);
        context
    };
    model.snapshot.selected = Some(context);
    model.snapshot.content = if local {
        ContentState::Offline
    } else {
        ContentState::Ready
    };
    model.snapshot.messages = page([message]);
    model.snapshot.message_panel = Some(talk_core::ui::messaging::MessagePanel::new(context));
    model.snapshot.content_panel = Default::default();
    (model, context)
}
fn thumbnail(message: u64) -> CacheThumbnail {
    let source = super::super::super::super::fixtures::snapshot(Some("chat-content"));
    let rich = &source.messages.as_slice()[0].metadata.rich.as_slice()[0];
    CacheThumbnail {
        room: stable(),
        message,
        placeholder: text("photo"),
        bytes: png().into(),
        saved_at: history::now_unix(),
        expires_at: None,
        attachment: talk_adapters::identity::CachedAttachment {
            placeholder: rich.placeholder.clone(),
            name: rich.name.clone(),
            mime_type: rich.mime_type.clone(),
            size: rich.size,
            id: rich.id.clone(),
        },
    }
}
fn image_job(context: ConversationRef, cached: bool) -> ImageJob {
    ImageJob {
        preview_generation: None,
        attachment: Some(thumbnail(7).attachment),
        cached,
        account: context.account,
        target: AssetTarget::Thumbnail(AttachmentRef {
            context,
            message: 7,
            placeholder: text("photo"),
        }),
        key: 42,
        bytes: ImageBytes::new(png()).unwrap(),
    }
}

#[test]
fn offline_hydration_is_bounded_and_binds_only_cached_thumbnail_jobs_without_network_authority() {
    let (mut model, context) = prepare(true);
    let first = model.snapshot.messages.as_slice()[0].clone();
    model.snapshot.messages = page((7..=9).map(|id| {
        let mut m = first.clone();
        m.local_id = id;
        m
    }));
    model.cache.accounts[0]
        .snapshot
        .as_mut()
        .unwrap()
        .thumbnails = page((7..=9).map(thumbnail));
    model.hydrate_thumbnails();
    assert_eq!(model.snapshot.content_panel.account, Some(context.account));
    assert_eq!(model.snapshot.content_panel.context, Some(context));
    assert_eq!(model.snapshot.content, ContentState::Offline);
    assert!(model.pending.is_empty());
    assert!(model.current(context.account).is_err());
    assert_eq!(
        model.cache.context(context.account.account, &stable()),
        None,
        "stable cache binding cannot become protocol authority"
    );
    let job = model.take_content_image().unwrap();
    let second = model.take_content_image().unwrap();
    assert!(
        model.take_content_image().is_none(),
        "at most two decode jobs are queued"
    );
    assert!(job.cached);
    assert_eq!(job.target.context(), Some(context));
    assert!(
        model.finish_content_image(&job, Some(4)),
        "ordinary cached thumbnail pixels are admitted despite unknown network permissions"
    );
    assert!(model.finish_content_image(&second, Some(4)));
    model.hydrate_thumbnails();
    assert_eq!(
        model.take_content_image().unwrap().target,
        AssetTarget::Thumbnail(AttachmentRef {
            context,
            message: 9,
            placeholder: text("photo")
        })
    );
    assert!(model.take_content_image().is_none());
}

#[test]
fn thumbnail_hydration_and_decode_completion_reject_denial_expiry_and_descriptor_revocation() {
    let (mut model, context) = prepare(true);
    model.cache.accounts[0]
        .snapshot
        .as_mut()
        .unwrap()
        .thumbnails = page([thumbnail(7)]);
    model.snapshot.content_panel.account = Some(context.account);
    let target = image_job(context, true).target;
    model.set_asset(AssetView {
        target: target.clone(),
        state: AssetState::Denied,
        emoji: None,
        color: None,
        document_pages: None,
    });
    model.hydrate_thumbnails();
    assert!(
        model.take_content_image().is_none(),
        "disk pixels never override explicit denial"
    );
    model.snapshot.content_panel.assets = Page::empty();
    model.hydrate_thumbnails();
    let job = model.take_content_image().unwrap();
    model.snapshot.messages = page(model.snapshot.messages.as_slice().iter().cloned().map(
        |mut m| {
            m.metadata.rich = Page::empty();
            m
        },
    ));
    assert!(
        !model.finish_content_image(&job, Some(4)),
        "a revoked descriptor cannot reappear after async decode"
    );
    model.snapshot.content_panel.assets = Page::empty();
    model.hydrate_thumbnails();
    assert!(model.take_content_image().is_none());
}

#[test]
fn accepted_network_thumbnail_updates_bounded_memory_and_coalesces_disk_write_without_cached_republication()
 {
    let (mut model, context) = prepare(false);
    let job = image_job(context, false);
    model.snapshot.content_panel.account = Some(context.account);
    model.set_asset(AssetView {
        target: job.target.clone(),
        state: AssetState::Ready { key: job.key },
        emoji: None,
        color: None,
        document_pages: None,
    });
    model.persist_thumbnail(&job, png());
    model.persist_thumbnail(&job, png());
    assert_eq!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .thumbnails
            .as_slice()
            .len(),
        1
    );
    assert_eq!(
        model
            .cache
            .writes
            .iter()
            .filter(|w| matches!(w.command, DataCommand::PutThumbnail(_)))
            .count(),
        1
    );
    let mut cached = job.clone();
    cached.cached = true;
    model.persist_thumbnail(&cached, png());
    assert_eq!(
        model
            .cache
            .writes
            .iter()
            .filter(|w| matches!(w.command, DataCommand::PutThumbnail(_)))
            .count(),
        1
    );
    model.invalidate_cached_room(context);
    assert!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
    assert!(
        !model
            .cache
            .writes
            .iter()
            .any(|w| matches!(w.command, DataCommand::PutThumbnail(_)))
    );
    let mut late = disk_snapshot();
    late.thumbnails = page([thumbnail(7)]);
    model.complete_data(
        Work::Load(context.account.account, 1),
        Ok(RuntimeReply::AccountData {
            account: context.account.account,
            result: Ok(DataResult::Snapshot(late)),
        }),
    );
    assert!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty(),
        "late disk load cannot restore revoked room pixels"
    );
}

#[test]
fn disposable_thumbnail_writes_cannot_evict_queued_room_or_history_metadata() {
    let (mut model, context) = prepare(false);
    model.cache.writes.clear();
    for id in 0..WRITES {
        model.cache.enqueue(
            context.account.account,
            DataCommand::MemberCount {
                room: NavigationRoomKey::new(&format!("r1:{:032x}", id)).unwrap(),
                count: 1,
            },
        );
    }
    assert_eq!(model.cache.writes.len(), WRITES);
    model.cache.enqueue(
        context.account.account,
        DataCommand::PutThumbnail(thumbnail(7)),
    );
    assert_eq!(model.cache.writes.len(), WRITES);
    assert!(
        model
            .cache
            .writes
            .iter()
            .all(|w| matches!(w.command, DataCommand::MemberCount { .. }))
    );
}

#[test]
fn exact_thumbnail_denial_fences_late_disk_load_and_bounded_overload_retains_required_removal() {
    let (mut model, context) = prepare(false);
    model.cache.accounts[0]
        .snapshot
        .as_mut()
        .unwrap()
        .thumbnails = page([thumbnail(7)]);
    let target = AttachmentRef {
        context,
        message: 7,
        placeholder: text("photo"),
    };
    model.forget_thumbnail(&target);
    assert!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
    let mut late = disk_snapshot();
    late.thumbnails = page([thumbnail(7)]);
    model.complete_data(
        Work::Load(context.account.account, 1),
        Ok(RuntimeReply::AccountData {
            account: context.account.account,
            result: Ok(DataResult::Snapshot(late)),
        }),
    );
    assert!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .thumbnails
            .as_slice()
            .is_empty()
    );
    for message in 8..=80 {
        model.forget_thumbnail(&AttachmentRef {
            context,
            message,
            placeholder: text("photo"),
        });
    }
    assert!(model.cache.writes.len() <= WRITES);
    assert!(
        model
            .cache
            .writes
            .iter()
            .any(|w| matches!(w.command, DataCommand::ClearThumbnails))
    );
    assert!(model.cache.thumbnail_denials.len() <= 64);
    assert!(model.cache.thumbnail_load_blocked.len() <= 8);
    assert!(
        model
            .cache
            .writes
            .iter()
            .any(|w| matches!(w.command, DataCommand::PutRooms(_))),
        "required room policy retained under removal pressure"
    );
}

#[test]
fn same_placeholder_replacement_and_changed_encoded_source_reject_obsolete_decode() {
    let (mut model, context) = prepare(true);
    model.cache.accounts[0]
        .snapshot
        .as_mut()
        .unwrap()
        .thumbnails = page([thumbnail(7)]);
    model.hydrate_thumbnails();
    let job = model.take_content_image().unwrap();
    assert!(model.cached_thumbnail_job_current(&job));
    model.snapshot.messages = page(model.snapshot.messages.as_slice().iter().cloned().map(
        |mut m| {
            m.metadata.rich = page(m.metadata.rich.as_slice().iter().cloned().map(|mut rich| {
                rich.id = Some(text("replacement"));
                rich
            }));
            m
        },
    ));
    assert!(!model.cached_thumbnail_job_current(&job));
    // A newly verified descriptor can carry a new raster at the same target.
    // Neither its presence nor the unchanged placeholder validates old bytes.
    let mut current = thumbnail(7);
    current.attachment.id = Some(text("replacement"));
    current.bytes = vec![1, 2, 3].into();
    model.cache.accounts[0]
        .snapshot
        .as_mut()
        .unwrap()
        .thumbnails = page([current]);
    assert!(!model.cached_thumbnail_job_current(&job));
    assert_eq!(job.target.context(), Some(context));
}

#[test]
fn video_and_pdf_thumbnail_rasters_hydrate_without_original_or_network_admission() {
    for mime in ["video/mp4", "application/pdf"] {
        let (mut model, context) = prepare(true);
        model.snapshot.messages = page(model.snapshot.messages.as_slice().iter().cloned().map(
            |mut m| {
                m.metadata.rich =
                    page(m.metadata.rich.as_slice().iter().cloned().map(|mut rich| {
                        rich.mime_type = Some(text(mime));
                        rich
                    }));
                m
            },
        ));
        let mut preview = thumbnail(7);
        preview.attachment.mime_type = Some(text(mime));
        model.cache.accounts[0]
            .snapshot
            .as_mut()
            .unwrap()
            .thumbnails = page([preview]);
        model.hydrate_thumbnails();
        let job = model.take_content_image().unwrap();
        assert!(job.cached);
        assert!(matches!(job.target, AssetTarget::Thumbnail(_)));
        assert!(model.cached_thumbnail_job_current(&job));
        assert!(model.pending.is_empty());
        assert!(model.current(context.account).is_err());
    }
}

fn dispatch_cached_write(model: &mut Accounts, at: Instant) -> Write {
    let index = model.cache.write_ready(at).unwrap();
    let write = model.cache.writes.remove(index);
    model.cache.in_flight = Some(write.clone());
    write
}
#[test]
fn failed_required_thumbnail_cleanup_retries_after_cooldown_and_only_saved_clears_failure() {
    let (mut model, context) = prepare(false);
    model.cache.writes.clear();
    model.cache.enqueue(
        context.account.account,
        DataCommand::RemoveThumbnail {
            room: stable(),
            message: 7,
            placeholder: text("photo"),
        },
    );
    let work = dispatch_cached_write(&mut model, Instant::now());
    model.complete_data(
        Work::Write(work.account, work.generation),
        Ok(RuntimeReply::AccountData {
            account: work.account,
            result: Err(talk_adapters::identity::DataError::Unavailable),
        }),
    );
    assert_eq!(model.cache.cleanup_failures.len(), 1);
    assert_eq!(model.cache.writes.len(), 1);
    assert!(matches!(
        model.cache.writes[0].command,
        DataCommand::ClearThumbnails
    ));
    let deadline = model.cache.cleanup_failures[0].retry_at;
    assert_eq!(
        model.cache.write_ready(deadline - Duration::from_millis(1)),
        None
    );
    assert_eq!(model.cache.write_ready(deadline), Some(0));
    assert!(
        !model
            .cache
            .cleanup_failed(work.account, work.generation, Instant::now()),
        "repeated failures do not start another diagnostic episode"
    );
    let deadline = model.cache.cleanup_failures[0].retry_at;
    let retry = dispatch_cached_write(&mut model, deadline);
    assert!(matches!(retry.command, DataCommand::ClearThumbnails));
    model.complete_data(
        Work::Write(retry.account, retry.generation),
        Ok(RuntimeReply::AccountData {
            account: retry.account,
            result: Ok(DataResult::Saved),
        }),
    );
    assert!(model.cache.cleanup_failures.is_empty());
    assert!(model.cache.writes.is_empty());
}

#[test]
fn retiring_profile_generation_cancels_cleanup_retry_and_late_failure_cannot_recreate_it() {
    let (mut model, context) = prepare(false);
    model.cache.writes.clear();
    model
        .cache
        .enqueue(context.account.account, DataCommand::ClearThumbnails);
    let write = dispatch_cached_write(&mut model, Instant::now());
    model.complete_data(
        Work::Write(write.account, write.generation),
        Err(IdentityError::WorkerUnavailable),
    );
    assert_eq!(model.cache.cleanup_failures.len(), 1);
    model.sync_cache_accounts(&connection_state(None, true));
    assert!(model.cache.cleanup_failures.is_empty());
    assert!(model.cache.writes.is_empty());
    assert!(model.cache.accounts.is_empty());
    model.complete_data(
        Work::Write(write.account, write.generation),
        Err(IdentityError::WorkerUnavailable),
    );
    assert!(model.cache.cleanup_failures.is_empty());
    model.sync_cache_accounts(&connection_state(Some(context.account), false));
    let current = model.cache.accounts[0].generation;
    assert_ne!(current, write.generation);
    assert!(
        !model
            .cache
            .cleanup_failed(write.account, write.generation, Instant::now())
    );
    assert!(model.cache.writes.is_empty());
}

#[test]
fn cleanup_failure_cooldown_preserves_bounded_metadata_outbox_and_other_ready_work() {
    use talk_adapters::identity::{OutboxEntry, OutboxIntent, OutboxRecord};
    let (mut model, context) = prepare(false);
    let entry = OutboxEntry {
        ordinal: 1,
        record: OutboxRecord::from_intent(
            OperationId::from_local(7),
            stable(),
            &OutboxIntent::Delete { message: 7 },
        )
        .unwrap(),
        saved_at: history::now_unix(),
    };
    model.cache.accounts[0].snapshot.as_mut().unwrap().outbox = page([entry.clone()]);
    model.cache.writes.clear();
    model.cache.enqueue(
        context.account.account,
        DataCommand::PutRooms(page([metadata()])),
    );
    for id in 0..WRITES - 1 {
        model.cache.enqueue(
            context.account.account,
            DataCommand::MemberCount {
                room: NavigationRoomKey::new(&format!("r1:{id:032x}")).unwrap(),
                count: 1,
            },
        );
    }
    assert!(
        model
            .cache
            .cleanup_failed(context.account.account, 1, Instant::now())
    );
    assert!(model.cache.writes.len() <= WRITES);
    assert!(
        model
            .cache
            .writes
            .iter()
            .any(|w| matches!(w.command, DataCommand::PutRooms(_)))
    );
    assert_eq!(
        model
            .cache
            .writes
            .iter()
            .filter(|w| required_cleanup(&w.command))
            .count(),
        1
    );
    let ready = model.cache.write_ready(Instant::now()).unwrap();
    assert!(
        !required_cleanup(&model.cache.writes[ready].command),
        "one account's cleanup cooldown cannot stall other cache observations"
    );
    assert_eq!(
        model
            .cache
            .loaded(context.account.account)
            .unwrap()
            .outbox
            .as_slice(),
        &[entry]
    );
    let deadline = model.cache.cleanup_failures[0].retry_at;
    assert!(required_cleanup(
        &model.cache.writes[model.cache.write_ready(deadline).unwrap()].command
    ));
}
