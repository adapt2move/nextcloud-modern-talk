use super::*;
use talk_core::ids::ConversationId;

fn listed(context: ConversationRef) -> protocol::Conversation {
    protocol::Conversation {
        id: context.conversation,
        call_scope: None,
        navigation_key: None,
        favorite: Some(false),
        organization: protocol::ConversationOrganization {
            favorites: protocol::FeatureSupport::Supported,
            favorite_change: Decision::Allowed,
            tags: protocol::FeatureSupport::Unsupported,
            tag_change: Decision::Denied,
            classified: Some(false),
            channel: Some(false),
            voice_room: Some(false),
        },
        tag_ids: None,
        name: text("Synthetic room"),
        kind: RoomKind::Group,
        role: protocol::Role::Moderator,
        archived: Some(false),
        archive: Decision::Allowed,
        actions: protocol::RoomActions {
            read_messages: Decision::Allowed,
            send_message: Decision::Allowed,
            list_participants: Decision::Allowed,
            leave_conversation: Decision::Allowed,
            join_call: Decision::Unknown,
        },
        unread_messages: Some(0),
        last_activity: Some(1),
        call_active: Some(false),
        sensitive: false,
        classified: false,
        channel: false,
        unknown_restrictions: false,
    }
}
fn list(
    context: ConversationRef,
    rooms: impl IntoIterator<Item = protocol::Conversation>,
) -> protocol::ConversationList {
    protocol::ConversationList {
        scope: context.account,
        rooms: page(rooms),
    }
}
fn details(room: protocol::Conversation) -> wire::ConversationDetails {
    wire::ConversationDetails {
        conversation: room,
        description: Some(text("Known description")),
        read_only: Some(false),
        notifications: Some(wire::NotificationLevel::Mention),
        call_notifications: Some(true),
        important: Some(true),
        preserved: Some(false),
        message_expiration: Some(3600),
        listable: None,
        mention_all: None,
        default_permissions: Some(255),
        server_default_permissions: Some(255),
        effective_permissions: Some(255),
        lobby: Some(false),
        lobby_timer: None,
        recording_consent: None,
        has_password: Some(true),
        participant_count: Some(4),
        controls: page(
            [wire::Setting::Metadata, wire::Setting::Members].map(|setting| {
                (
                    setting,
                    wire::ActionAvailability {
                        implemented: true,
                        server: protocol::FeatureSupport::Supported,
                        allowed: Decision::Allowed,
                    },
                )
            }),
        ),
    }
}
fn known(now: Instant) -> (Accounts, ConversationRef, protocol::Conversation) {
    let mut model = super::super::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
    let context = model.snapshot.selected.unwrap();
    let room = listed(context);
    let value = details_view(details(room.clone()));
    model.background.sync(&[context.account], now);
    let account = model.background.account_mut(context.account).unwrap();
    account
        .metadata
        .reconcile(context.account, &list(context, [room.clone()]));
    account.metadata.remember(context, &value, true, now);
    account.metadata.rooms[0].dirty = false;
    let mut panel = view::ConversationPanel::new(context);
    panel.details = Some(value);
    model.snapshot.conversation_panel = Some(panel);
    (model, context, room)
}

#[test]
fn newly_created_room_can_track_its_first_detail_read_before_list_refresh() {
    let now = Instant::now();
    let (mut model, old, _) = known(now);
    let context = ConversationRef {
        conversation: ConversationId {
            local: 777,
            ..old.conversation
        },
        ..old
    };
    let room = listed(context);
    model.snapshot.conversations = page(
        std::iter::once(conversation(&room, context.account))
            .chain(model.snapshot.conversations.as_slice().iter().cloned()),
    );
    model.observe_created_room(context.account, room.clone());
    let metadata = &mut model
        .background
        .account_mut(context.account)
        .unwrap()
        .metadata;
    assert!(metadata.needs_read(context, now));
    let operation = OperationId::from_local(91);
    metadata.admitted(context, operation, true);
    assert!(
        metadata.complete(context, operation),
        "the first creation read has a registered owner before the HTTP list arrives"
    );
    assert_eq!(metadata.rooms.len(), 2);
    metadata.reconcile(context.account, &list(context, [room]));
    assert_eq!(metadata.rooms.len(), 1);
}
#[test]
fn unread_activity_and_call_only_lists_do_not_request_details_or_renew_the_source() {
    let now = Instant::now();
    let (mut model, context, mut room) = known(now);
    let before = model.snapshot.conversation_panel.clone();
    room.unread_messages = Some(20);
    room.last_activity = Some(999);
    room.call_active = Some(true);
    model.apply_room_refresh(
        context.account,
        Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(list(
            context,
            [room.clone()],
        )))),
    );
    let metadata = &model.background.account(context.account).unwrap().metadata;
    assert!(!metadata.needs_read(context, now + Duration::from_secs(59)));
    assert!(
        metadata.rooms[0].attempted.is_none(),
        "no Read admission was attempted"
    );
    assert_eq!(metadata.rooms[0].observed, Some(now));
    assert_eq!(model.snapshot.conversation_panel, before);
    assert!(model.pending.is_empty());

    room.name = text("Changed server name");
    model.apply_room_refresh(
        context.account,
        Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(list(
            context,
            [room],
        )))),
    );
    let metadata = &model.background.account(context.account).unwrap().metadata;
    assert!(
        metadata.rooms[0].attempted.is_some(),
        "a relevant change attempts a Read"
    );
    assert_eq!(
        model.snapshot.conversation_panel, before,
        "failed admission is quiet"
    );
}

#[test]
fn fallback_retry_and_hint_bursts_share_one_bounded_observation() {
    let now = Instant::now();
    let (mut model, context, room) = known(now);
    let metadata = &mut model
        .background
        .account_mut(context.account)
        .unwrap()
        .metadata;
    assert!(metadata.needs_read(context, now + METADATA_LIFETIME));
    metadata.attempted(context, now + METADATA_LIFETIME);
    assert!(!metadata.needs_read(
        context,
        now + METADATA_LIFETIME + METADATA_RETRY - Duration::from_millis(1)
    ));
    assert!(metadata.needs_read(context, now + METADATA_LIFETIME + METADATA_RETRY));
    let operation = OperationId::from_local(41);
    metadata.admitted(context, operation, true);
    for _ in 0..100 {
        metadata.hint();
    }
    assert!(
        !metadata.complete(context, operation),
        "a hint already retires the older Read before HTTP reconciliation"
    );
    model.apply_room_refresh(
        context.account,
        Err(IdentityError::Request(ErrorKind::Transport)),
    );
    let metadata = &mut model
        .background
        .account_mut(context.account)
        .unwrap()
        .metadata;
    assert!(metadata.hinted, "failed lists cannot consume the event");
    assert_eq!(
        metadata.reconcile(context.account, &list(context, [room])),
        vec![(context, false)]
    );
    assert!(metadata.needs_read(context, now + METADATA_LIFETIME + METADATA_RETRY));
    assert_eq!(metadata.rooms.len(), 1);
    assert!(model.pending.is_empty());
}

#[test]
fn newer_policy_or_metadata_retires_old_reads_but_counters_do_not() {
    let now = Instant::now();
    for change in 0..4 {
        let (mut model, context, mut room) = known(now);
        let operation = OperationId::from_local(42);
        model
            .background
            .account_mut(context.account)
            .unwrap()
            .metadata
            .admitted(context, operation, true);
        match change {
            0 => room.unread_messages = Some(12),
            1 => room.name = text("Renamed"),
            2 => room.role = protocol::Role::Member,
            _ => room.actions.read_messages = Decision::Denied,
        }
        model.apply_room_refresh(
            context.account,
            Ok(RuntimeReply::Protocol(ProtocolReply::Conversations(list(
                context,
                [room],
            )))),
        );
        let metadata = &mut model
            .background
            .account_mut(context.account)
            .unwrap()
            .metadata;
        assert_eq!(metadata.complete(context, operation), change == 0);
        let panel = model.snapshot.conversation_panel.as_ref().unwrap();
        assert_eq!(
            panel
                .details
                .as_ref()
                .unwrap()
                .availability(view::RoomSetting::Metadata)
                .enabled(),
            change < 2
        );
        if change >= 2 {
            assert!(panel.details_stale);
            let before = panel.clone();
            model.complete_room(
                RequestId(0),
                context,
                operation,
                Work::Revalidate,
                Ok(RuntimeReply::Protocol(
                    ProtocolReply::ConversationSettings {
                        conversation: context.conversation,
                        operation,
                        result: wire::ConversationResult::Details(Box::new(details(listed(
                            context,
                        )))),
                    },
                )),
            );
            assert_eq!(model.snapshot.conversation_panel.as_ref().unwrap(), &before);
        }
    }
}

#[test]
fn explicit_reload_settles_uncertainty_while_background_failures_remain_quiet() {
    let now = Instant::now();
    for work in [Work::Revalidate, Work::ReadKnown] {
        let (mut model, context, room) = known(now);
        let panel = model.snapshot.conversation_panel.as_mut().unwrap();
        panel.state = view::RemoteState::Uncertain(RequestId(7));
        let result = view::RoomResult {
            request: RequestId(7),
            completion: Completion::Uncertain,
        };
        panel.result = Some(result);
        model.apply_room_result(
            RequestId(8),
            context,
            work,
            Ok(wire::ConversationResult::Details(Box::new(details(room)))),
        );
        let panel = model.snapshot.conversation_panel.as_ref().unwrap();
        assert_eq!(
            panel.state,
            if matches!(work, Work::ReadKnown) {
                view::RemoteState::Idle
            } else {
                view::RemoteState::Uncertain(RequestId(7))
            }
        );
        assert_eq!(
            panel.result,
            Some(result),
            "reconciliation does not acknowledge the original outcome"
        );
    }
    for work in [Work::Revalidate, Work::MemberCount] {
        let (mut model, context, _) = known(now);
        let before = model.snapshot.conversation_panel.clone();
        for reason in [
            UnavailableReason::Busy,
            UnavailableReason::Connection(ConnectionFailure::Unreachable),
        ] {
            model.apply_room_result(RequestId(9), context, work, Err(Completion::Failed(reason)));
            assert_eq!(
                model.snapshot.conversation_panel, before,
                "background contention/transport failure cannot add a status footer"
            );
        }
    }
}
#[test]
fn quiet_revalidation_keeps_controls_values_and_unacknowledged_write_failures() {
    let now = Instant::now();
    for work in [Work::Revalidate, Work::ReadKnown] {
        let (mut model, context, room) = known(now);
        let failure = Completion::Failed(UnavailableReason::Unsupported);
        let panel = model.snapshot.conversation_panel.as_mut().unwrap();
        panel.state = view::RemoteState::Failed(RequestId(8), UnavailableReason::Unsupported);
        panel.result = Some(view::RoomResult {
            request: RequestId(8),
            completion: failure,
        });
        let before = panel.clone();
        begin_room(panel, RequestId(9), work);
        assert_eq!(panel, &before);
        let mut fresh = details(room);
        fresh.description = Some(text("Updated description"));
        model.apply_room_result(
            RequestId(9),
            context,
            work,
            Ok(wire::ConversationResult::Details(Box::new(fresh))),
        );
        let panel = model.snapshot.conversation_panel.as_ref().unwrap();
        assert_eq!(panel.state, before.state);
        assert_eq!(panel.result, before.result);
        assert_eq!(
            panel
                .details
                .as_ref()
                .unwrap()
                .description
                .as_ref()
                .unwrap()
                .as_str(),
            "Updated description"
        );
        assert!(
            panel
                .details
                .as_ref()
                .unwrap()
                .availability(view::RoomSetting::Metadata)
                .enabled()
        );
        assert!(!panel.details_stale);
        model.apply_room_result(
            RequestId(10),
            context,
            work,
            Err(Completion::Failed(UnavailableReason::Connection(
                ConnectionFailure::Unreachable,
            ))),
        );
        assert_eq!(
            model.snapshot.conversation_panel.as_ref().unwrap().state,
            before.state
        );
        model.apply_room_result(
            RequestId(11),
            context,
            work,
            Err(Completion::Failed(UnavailableReason::PermissionDenied)),
        );
        let panel = model.snapshot.conversation_panel.as_ref().unwrap();
        assert!(
            !panel
                .details
                .as_ref()
                .unwrap()
                .availability(view::RoomSetting::Metadata)
                .enabled()
        );
        assert_eq!(panel.details.as_ref().unwrap().leave, Decision::Denied);
        assert_eq!(panel.result, before.result);
    }
    let context = crate::composition::fixtures::context(1);
    let mut initial = view::ConversationPanel::new(context);
    begin_room(&mut initial, RequestId(12), Work::Read);
    assert_eq!(initial.state, view::RemoteState::Loading(RequestId(12)));
    assert!(initial.details.is_none());
}

#[test]
fn navigation_retains_only_readonly_room_values_and_runtime_replacement_drops_tickets() {
    let now = Instant::now();
    let (mut model, context, room) = known(now);
    let mut other = context;
    other.conversation.local += 1;
    let other_room = listed(other);
    model
        .background
        .account_mut(context.account)
        .unwrap()
        .metadata
        .reconcile(
            context.account,
            &list(context, [room.clone(), other_room.clone()]),
        );
    model.snapshot.conversations = page([
        conversation(&room, context.account),
        conversation(&other_room, context.account),
    ]);
    model.select_conversation(other).unwrap();
    assert!(
        model
            .snapshot
            .conversation_panel
            .as_ref()
            .unwrap()
            .details
            .is_none()
    );
    model.select_conversation(context).unwrap();
    let panel = model.snapshot.conversation_panel.as_ref().unwrap();
    assert!(panel.details_stale);
    let value = panel.details.as_ref().unwrap();
    assert_eq!(
        value.description.as_ref().unwrap().as_str(),
        "Known description"
    );
    assert!(value.controls.as_slice().is_empty());
    assert_eq!(value.leave, Decision::Unknown);
    assert_eq!(value.default_permissions, None);
    assert_eq!(value.server_default_permissions, None);
    assert_eq!(value.effective_permissions, None);
    assert_eq!(value.has_password, None);
    let mut replacement = context.account;
    replacement.runtime = talk_core::ids::RuntimeGeneration::from_local(99);
    model.background.sync(&[replacement], now);
    assert!(model.background.account(context.account).is_none());
    assert!(
        model
            .background
            .account(replacement)
            .unwrap()
            .metadata
            .rooms
            .is_empty()
    );
}

#[test]
fn metadata_retention_is_bounded_to_current_room_scope_and_removal_retires_operations() {
    let now = Instant::now();
    let (mut model, context, room) = known(now);
    let metadata = &mut model
        .background
        .account_mut(context.account)
        .unwrap()
        .metadata;
    let rooms = (1..=100).map(|local| {
        let mut room = room.clone();
        room.id.local = local;
        room
    });
    let full = list(context, rooms);
    metadata.reconcile(context.account, &full);
    assert_eq!(metadata.rooms.len(), 100);
    let context = metadata.rooms[0].context;
    metadata.admitted(context, OperationId::from_local(5), true);
    metadata.reconcile(context.account, &list(context, []));
    assert!(metadata.rooms.is_empty());
    assert!(!metadata.complete(context, OperationId::from_local(5)));
}
