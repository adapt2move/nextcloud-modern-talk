use super::*;

fn observation(at: u64) -> CacheRoomObservation {
    CacheRoomObservation {
        description: Some(text("Cached description")),
        read_only: Some(true),
        notifications: Some(CachedNotificationLevel::Mention),
        call_notifications: Some(false),
        important: Some(true),
        preserved: Some(false),
        message_expiration: Some(3600),
        lobby: Some(true),
        guest_access: Some(false),
        observed_at: at,
    }
}
fn cached(model: &Accounts) -> &CacheRoom {
    &model.cache.accounts[0]
        .snapshot
        .as_ref()
        .unwrap()
        .rooms
        .as_slice()[0]
}

#[test]
fn restart_observations_restore_readonly_and_activity_does_not_renew_detail_age() {
    let mut model = super::super::super::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
    let scope = model.snapshot.active_account.unwrap();
    install(&mut model, scope);
    let at = history::now_unix().saturating_sub(500);
    model.cache.accounts[0].snapshot.as_mut().unwrap().rooms = page([CacheRoom {
        observation: Some(observation(at)),
        ..metadata()
    }]);
    model.background.sync(&[scope], Instant::now());
    model.restore_cache_views();
    let local = model.snapshot.conversations.as_slice()[0].context;
    model.select_conversation(local).unwrap();
    let panel = model.snapshot.conversation_panel.as_ref().unwrap();
    assert!(panel.details_stale);
    let details = panel.details.as_ref().unwrap();
    assert_eq!(
        details.description.as_ref().unwrap().as_str(),
        "Cached description"
    );
    assert_eq!(
        details.notifications,
        Some(talk_core::ui::conversation::NotificationLevel::Mention)
    );
    assert!(details.controls.as_slice().is_empty());
    assert_eq!(details.leave, Decision::Unknown);
    assert_eq!(
        (
            details.default_permissions,
            details.server_default_permissions,
            details.effective_permissions
        ),
        (None, None, None)
    );
    assert_eq!(details.has_password, None);
    assert!(
        model
            .room_action(
                RequestId(1),
                local,
                talk_core::ui::conversation::ConversationAction::Read
            )
            .is_err()
    );
    assert!(model.pending.is_empty());

    let mut room = observed(scope);
    let live = ConversationRef {
        account: scope,
        conversation: room.id,
    };
    for activity in [2, 10, 999] {
        room.last_activity = Some(activity);
        room.unread_messages = Some(activity as u32);
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room.clone()]),
            },
        );
        assert_eq!(cached(&model).observation.as_ref().unwrap().observed_at, at);
    }
    model.restore_room_metadata(live);
    assert!(
        model
            .snapshot
            .conversation_panel
            .as_ref()
            .unwrap()
            .details_stale
    );
    assert!(
        model.cached_room_details(local).is_none(),
        "the retired synthetic scope cannot read the live binding"
    );
}

#[test]
fn detail_retirement_fences_late_disk_reads_and_obsolete_scope_cleanup() {
    let mut model = super::super::super::tests::model();
    let scope = crate::composition::fixtures::context(1).account;
    install(&mut model, scope);
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
    let mut disk = disk_snapshot();
    disk.rooms = page([CacheRoom {
        observation: Some(observation(history::now_unix())),
        ..metadata()
    }]);
    model.complete_data(
        Work::Load(scope.account, 1),
        Ok(RuntimeReply::AccountData {
            account: scope.account,
            result: Ok(DataResult::Snapshot(disk.clone())),
        }),
    );
    assert!(
        model.cached_room_details(context).is_some(),
        "late initial load fills eligible scalar observations"
    );
    let mut obsolete = context;
    obsolete.account.runtime = RuntimeGeneration::from_local(99);
    model.forget_cached_room_details(obsolete);
    assert!(model.cached_room_details(context).is_some());
    model.forget_cached_room_details(context);
    assert!(cached(&model).observation.is_none());
    model.complete_data(
        Work::Load(scope.account, 1),
        Ok(RuntimeReply::AccountData {
            account: scope.account,
            result: Ok(DataResult::Snapshot(disk)),
        }),
    );
    assert!(model.cached_room_details(context).is_none());
    assert!(
        cached(&model).observation.is_none(),
        "a late disk read cannot resurrect denied observations"
    );
    assert!(
        matches!(&model.cache.writes.last().unwrap().command, DataCommand::PutRooms(rooms) if rooms.as_slice()[0].observation.is_none())
    );
}

#[test]
fn restricted_unknown_and_denied_rooms_drop_scalar_observations_and_required_cleanup_is_bounded() {
    for restriction in 0..4 {
        let mut model = super::super::super::tests::model();
        let scope = crate::composition::fixtures::context(1).account;
        install(&mut model, scope);
        model.cache.accounts[0].snapshot.as_mut().unwrap().rooms = page([CacheRoom {
            observation: Some(observation(history::now_unix())),
            ..metadata()
        }]);
        let mut room = observed(scope);
        match restriction {
            0 => room.sensitive = true,
            1 => room.classified = true,
            2 => room.unknown_restrictions = true,
            _ => room.actions.read_messages = Decision::Denied,
        }
        model.observe_cache_rooms(
            scope,
            &protocol::ConversationList {
                scope,
                rooms: page([room]),
            },
        );
        assert!(
            model.cache.accounts[0]
                .snapshot
                .as_ref()
                .unwrap()
                .rooms
                .as_slice()
                .is_empty()
        );
        assert!(
            matches!(&model.cache.writes.last().unwrap().command, DataCommand::PutRooms(rooms) if rooms.as_slice().is_empty())
        );
        assert!(model.cache.writes.len() <= WRITES);
    }
}
