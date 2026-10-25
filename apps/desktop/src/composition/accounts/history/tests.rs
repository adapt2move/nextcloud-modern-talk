use super::*;
use talk_core::ids::{ConversationId, HostId, RuntimeGeneration};
use talk_core::protocol::messaging::{MessageDetails, ReplyContext};
use talk_core::protocol::{Message, MessageId, MessageKind};

#[test]
fn disk_warmup_does_not_invent_network_failure_or_replace_current_observations() {
    let original = context(1, 1, 1, 1);
    let mut source = Cache::default();
    source.open(original);
    complete(
        &mut source,
        original,
        1,
        Work::History,
        messages(original, [message(original, 7, "disk body")], None),
    );
    let durable = source
        .durable(
            original,
            NavigationRoomKey::new("r1:00000000000000000000000000000001").unwrap(),
            now_unix(),
        )
        .unwrap();
    let local = context(1, 0, 0, 1);
    let mut cached = Cache::default();
    cached.restore(local, &durable);
    assert!(!cached.get(local).unwrap().offline);
    let pending = context(1, 2, 1, 1);
    cached.open(pending);
    let operation = admit(&mut cached, pending, 1, Work::History);
    let state = cached.get(pending).unwrap().state;
    cached.restore(pending, &durable);
    assert_eq!(cached.get(pending).unwrap().state, state);
    assert_eq!(cached.flights.len(), 1);
    assert_eq!(
        cached.get(pending).unwrap().messages.as_slice()[0]
            .body
            .as_str(),
        "disk body"
    );
    assert!(cached.complete(
        pending,
        operation,
        Work::History,
        RequestId(1),
        &Ok(messages(pending, [message(pending, 7, "fresh body")], None)),
        false
    ));
    cached.restore(pending, &durable);
    assert_eq!(
        cached.get(pending).unwrap().messages.as_slice()[0]
            .body
            .as_str(),
        "fresh body"
    );
    let failed = context(1, 3, 1, 1);
    cached.open(failed);
    let operation = admit(&mut cached, failed, 2, Work::History);
    assert!(cached.complete(
        failed,
        operation,
        Work::History,
        RequestId(2),
        &Err(Completion::Failed(UnavailableReason::Unconfigured)),
        true
    ));
    cached.restore(failed, &durable);
    assert!(
        cached.get(failed).unwrap().offline,
        "a real observed transport failure survives cache warmup"
    );
    let edited = context(1, 4, 1, 1);
    cached.open(edited);
    complete(
        &mut cached,
        edited,
        3,
        Work::Action,
        MessagingResult::Message(Box::new(message(edited, 7, "fresh mutation"))),
    );
    cached.restore(edited, &durable);
    assert_eq!(
        cached.get(edited).unwrap().messages.as_slice()[0]
            .body
            .as_str(),
        "fresh mutation"
    );
}

#[test]
fn durable_roundtrip_preserves_confirmed_content_and_pins_without_permissions_or_live_tickets() {
    let key = context(1, 1, 1, 1);
    let mut original = Cache::default();
    original.open(key);
    let mut one = message(key, 1, "confirmed");
    one.edit = Decision::Allowed;
    one.delete = Decision::Allowed;
    complete(
        &mut original,
        key,
        1,
        Work::History,
        messages(key, [one], None),
    );
    let mut pinned = message_view(&message(key, 99, "older pinned excerpt"));
    pinned.metadata.edit = Decision::Allowed;
    original.get_mut(key).unwrap().pins = page([pinned]);
    original.get_mut(key).unwrap().pins_loaded = true;
    let mut pending = message_view(&message(key, 2, "not confirmed"));
    pending.delivery = talk_core::ui::presentation::DeliveryState::Pending;
    original
        .get_mut(key)
        .unwrap()
        .messages
        .try_push(pending)
        .unwrap();
    let stable = NavigationRoomKey::new("r1:00000000000000000000000000000001").unwrap();
    let durable = original.durable(key, stable, now_unix()).unwrap();
    assert_eq!(durable.messages.as_slice().len(), 1);
    let restored = context(1, 2, 0, 1);
    let mut cache = Cache::default();
    cache.restore(restored, &durable);
    let room = cache.get(restored).unwrap();
    assert!(room.loaded && room.pins_loaded);
    assert_eq!(room.messages.as_slice()[0].body.as_str(), "confirmed");
    assert_eq!(room.messages.as_slice()[0].metadata.edit, Decision::Unknown);
    assert_eq!(
        room.messages.as_slice()[0].metadata.delete,
        Decision::Unknown
    );
    assert_eq!(
        room.pins.as_slice()[0].body.as_str(),
        "older pinned excerpt"
    );
    assert!(room.capabilities.is_none() && cache.flights.is_empty());
    // Cached state cannot replace newer canonical data after a network read.
    complete(
        &mut cache,
        restored,
        2,
        Work::History,
        messages(restored, [message(restored, 1, "new canonical")], None),
    );
    cache.restore(restored, &durable);
    assert_eq!(
        cache.get(restored).unwrap().messages.as_slice()[0]
            .body
            .as_str(),
        "new canonical"
    );
}

fn context(account: u128, runtime: u128, host: u128, room: u128) -> ConversationRef {
    let account = AccountId::from_local(account);
    ConversationRef {
        account: AccountScope {
            account,
            runtime: RuntimeGeneration::from_local(runtime),
        },
        conversation: ConversationId {
            account,
            host: HostId::from_local(host),
            local: room,
        },
    }
}
fn message(context: ConversationRef, id: u64, body: &str) -> MessageDetails {
    MessageDetails {
        author_key: None,
        message: Message {
            id: MessageId {
                conversation: context.conversation,
                sequence: id,
            },
            author: text("Synthetic user"),
            text: text(body),
            kind: MessageKind::Comment,
            timestamp: 1,
            expires_at: None,
            edited_at: None,
            own_operation: None,
        },
        notify: Decision::Denied,
        own: false,
        replyable: Some(true),
        parent: None,
        thread: None,
        reactions: Page::empty(),
        rich: Page::empty(),
        system: None,
        edit: Decision::Unknown,
        delete: Decision::Unknown,
    }
}
fn messages(
    context: ConversationRef,
    rows: impl IntoIterator<Item = MessageDetails>,
    cursor: Option<u64>,
) -> MessagingResult {
    MessagingResult::Messages {
        capabilities: None,
        messages: page(rows),
        cursor: cursor.map(|sequence| MessageId {
            conversation: context.conversation,
            sequence,
        }),
        common_read: None,
    }
}
fn admit(cache: &mut Cache, context: ConversationRef, serial: u64, work: Work) -> OperationId {
    let operation = OperationId::from_local(u128::from(serial));
    cache.admitted(context, operation, work, RequestId(serial), serial);
    operation
}
fn complete(
    cache: &mut Cache,
    context: ConversationRef,
    serial: u64,
    work: Work,
    result: MessagingResult,
) {
    let operation = admit(cache, context, serial, work);
    assert!(cache.complete(
        context,
        operation,
        work,
        RequestId(serial),
        &Ok(result),
        false
    ));
}
fn select(model: &mut Accounts, context: ConversationRef) {
    model.select(Some(context.account));
    model.clear_content();
    model.snapshot.selected = Some(context);
    model.history.open(context);
    model.snapshot.message_panel = Some(MessagePanel::new(context));
    model.project_history(context);
}
// Model successful admission without starting the protocol worker. The same
// request/target projection is established by Accounts::start_message.
fn admit_target(
    model: &mut Accounts,
    key: ConversationRef,
    serial: u64,
    work: Work,
) -> OperationId {
    let operation = admit(&mut model.history, key, serial, work);
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    match work {
        Work::Thread => {
            panel.thread_root = Some(7);
            panel.thread_state = RemoteState::Loading(RequestId(serial));
        }
        Work::Context(target) => {
            panel.context_load = Some(talk_core::ui::messaging::ContextLoad {
                target,
                state: RemoteState::Loading(RequestId(serial)),
            });
            panel.located = None;
        }
        _ => panic!("targeted read required"),
    }
    operation
}

fn reply(
    context: ConversationRef,
    operation: OperationId,
    result: MessagingResult,
) -> Result<RuntimeReply, IdentityError> {
    Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
        conversation: context.conversation,
        operation,
        result,
    }))
}

#[test]
fn empty_room_background_reads_remain_ready_and_preserve_explicit_history_ownership() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    complete(&mut cache, key, 1, Work::History, messages(key, [], None));
    assert!(cache.get(key).unwrap().loaded);
    let latest = admit(&mut cache, key, 2, Work::Latest);
    assert_eq!(cache.get(key).unwrap().state, RemoteState::Idle);
    assert!(cache.complete(
        key,
        latest,
        Work::Latest,
        RequestId(2),
        &Ok(messages(key, [message(key, 7, "First note")], Some(7))),
        false,
    ));
    let room = cache.get(key).unwrap();
    assert_eq!(room.state, RemoteState::Idle);
    assert_eq!(room.latest, Some(7));
    assert_eq!(room.older, Some(7));
    let older = admit(&mut cache, key, 3, Work::Older);
    complete(&mut cache, key, 4, Work::Latest, messages(key, [], None));
    assert_eq!(
        cache.get(key).unwrap().state,
        RemoteState::Loading(RequestId(3))
    );
    let failure = Completion::Failed(UnavailableReason::Unconfigured);
    assert!(cache.complete(key, older, Work::Older, RequestId(3), &Err(failure), false));
    complete(&mut cache, key, 5, Work::Latest, messages(key, [], None));
    assert_eq!(
        cache.get(key).unwrap().state,
        RemoteState::Failed(RequestId(3), UnavailableReason::Unconfigured)
    );
}

#[test]
fn actual_account_runtime_host_and_room_keys_are_isolated_and_retired() {
    let a = context(1, 1, 1, 7);
    let b = context(2, 1, 1, 7);
    let a_new_runtime = context(1, 2, 1, 7);
    let a_other_host = context(1, 1, 2, 7);
    let a_other_room = context(1, 1, 1, 8);
    let mut cache = Cache::default();
    for (serial, key) in [a, b, a_new_runtime, a_other_host, a_other_room]
        .into_iter()
        .enumerate()
    {
        cache.open(key);
        let body = format!("Isolated content {serial}");
        complete(
            &mut cache,
            key,
            serial as u64 + 1,
            Work::History,
            messages(key, [message(key, 1, &body)], None),
        );
        assert_eq!(
            cache.get(key).unwrap().messages.as_slice()[0].body.as_str(),
            body
        );
    }
    cache.retain_accounts(&[a_new_runtime.account, b.account]);
    assert!(cache.get(a).is_none());
    assert!(cache.get(a_other_host).is_none());
    assert!(cache.get(a_other_room).is_none());
    assert!(cache.get(a_new_runtime).is_some());
    cache.remove_account(a.account.account);
    assert!(cache.get(a_new_runtime).is_none());
    assert_eq!(
        cache.get(b).unwrap().messages.as_slice()[0].body.as_str(),
        "Isolated content 1"
    );
}

#[test]
fn switched_room_completion_restores_warm_history_and_offline_refresh_keeps_it_readable() {
    let mut model = super::super::tests::model();
    let a = context(1, 1, 1, 7);
    let b = context(2, 1, 1, 7);
    select(&mut model, a);
    let op_a = admit(&mut model.history, a, 1, Work::History);
    select(&mut model, b);
    let op_b = admit(&mut model.history, b, 2, Work::History);
    model.apply_message(
        RequestId(1),
        a,
        op_a,
        Work::History,
        reply(
            a,
            op_a,
            messages(a, [message(a, 90, "Account A")], Some(90)),
        ),
    );
    assert!(model.snapshot.messages.as_slice().is_empty());
    select(&mut model, a);
    assert_eq!(model.snapshot.content, ContentState::Ready);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Account A"
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().older,
        Some(90)
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().latest,
        Some(90)
    );
    model.apply_message(
        RequestId(2),
        b,
        op_b,
        Work::History,
        reply(
            b,
            op_b,
            messages(b, [message(b, 90, "Account B")], Some(90)),
        ),
    );
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Account A"
    );
    let poll = admit(&mut model.history, a, 3, Work::Poll);
    model.apply_message(
        RequestId(3),
        a,
        poll,
        Work::Poll,
        Err(IdentityError::Request(ErrorKind::Transport)),
    );
    assert_eq!(model.snapshot.content, ContentState::Offline);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Account A"
    );
    assert!(matches!(
        model.snapshot.message_panel.as_ref().unwrap().history,
        RemoteState::Failed(..)
    ));
    select(&mut model, b);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Account B"
    );
}

#[test]
fn cached_offline_navigation_does_not_use_stale_send_permission_as_a_read_gate() {
    let mut model = super::super::tests::model();
    let key = context(1, 1, 1, 7);
    let fixture = super::super::super::fixtures::snapshot(Some("chat"));
    let mut row = fixture.conversations.as_slice()[0].clone();
    row.context = key;
    row.send = Decision::Stale;
    model.snapshot.active_account = Some(key.account);
    model.snapshot.conversations = page([row]);
    model.snapshot.selected = None;
    model.snapshot.conversation_list.content = ContentState::Offline;
    model.history.open(key);
    complete(
        &mut model.history,
        key,
        1,
        Work::History,
        messages(key, [message(key, 7, "Previously authorized")], None),
    );
    assert_eq!(model.select_conversation(key), Ok(true));
    assert_eq!(model.snapshot.content, ContentState::Ready);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Previously authorized"
    );
    assert_eq!(
        model.snapshot.conversations.as_slice()[0].send,
        Decision::Stale
    );
    assert!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .features
            .as_slice()
            .is_empty()
    );
    assert!(
        model.pending.is_empty(),
        "restoration itself performs no network request"
    );
    assert_eq!(
        model.select_conversation(context(2, 1, 1, 7)),
        Err(SubmitError::StaleScope)
    );
}

#[test]
fn refresh_deduplicates_without_resetting_older_and_forward_event_cursor_never_rewinds() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    complete(
        &mut cache,
        key,
        1,
        Work::History,
        messages(
            key,
            (101..=150).map(|id| message(key, id, "Recent")),
            Some(101),
        ),
    );
    complete(
        &mut cache,
        key,
        2,
        Work::Older,
        messages(
            key,
            (51..=100).map(|id| message(key, id, "Older")),
            Some(51),
        ),
    );
    complete(
        &mut cache,
        key,
        3,
        Work::History,
        messages(
            key,
            (101..=150).map(|id| message(key, id, "Refreshed")),
            Some(101),
        ),
    );
    let room = cache.get(key).unwrap();
    assert_eq!(room.messages.as_slice().len(), 100);
    assert_eq!(room.older, Some(51));
    assert_eq!(room.messages.as_slice()[0].local_id, 51);
    complete(
        &mut cache,
        key,
        4,
        Work::Poll,
        messages(key, [message(key, 151, "New")], Some(500)),
    );
    let room = cache.get(key).unwrap();
    assert_eq!(room.older, Some(52), "capacity eviction remains pageable");
    assert_eq!(room.latest, Some(500));
    complete(
        &mut cache,
        key,
        5,
        Work::Older,
        messages(key, (1..=50).map(|id| message(key, id, "Oldest")), None),
    );
    let room = cache.get(key).unwrap();
    assert_eq!(
        room.latest,
        Some(500),
        "backward pages must not use the visible tail as a poll cursor"
    );
    assert_eq!(room.older, None);
    assert_eq!(room.messages.as_slice().len(), 100);
    assert_eq!(room.versions.len(), 100);
}

#[test]
fn out_of_order_reads_and_reads_started_before_mutation_completion_cannot_regress_rows() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    complete(
        &mut cache,
        key,
        1,
        Work::History,
        messages(key, [message(key, 1, "Original")], None),
    );
    let older_read = admit(&mut cache, key, 2, Work::History);
    let new_read = admit(&mut cache, key, 3, Work::Poll);
    assert!(cache.complete(
        key,
        new_read,
        Work::Poll,
        RequestId(3),
        &Ok(messages(key, [message(key, 1, "Edited")], Some(300))),
        false
    ));
    assert!(cache.complete(
        key,
        older_read,
        Work::History,
        RequestId(2),
        &Ok(messages(key, [message(key, 1, "Stale original")], Some(1))),
        false
    ));
    assert_eq!(
        cache.get(key).unwrap().messages.as_slice()[0].body.as_str(),
        "Edited"
    );
    assert_eq!(cache.get(key).unwrap().latest, Some(300));
    let deletion = admit(&mut cache, key, 4, Work::Action);
    let overlapping_read = admit(&mut cache, key, 5, Work::Poll);
    let mut tombstone = message(key, 1, "Must be erased");
    tombstone.message.kind = MessageKind::Deleted;
    assert!(cache.complete(
        key,
        deletion,
        Work::Action,
        RequestId(4),
        &Ok(MessagingResult::Message(Box::new(tombstone))),
        false
    ));
    assert!(cache.complete(
        key,
        overlapping_read,
        Work::Poll,
        RequestId(5),
        &Ok(messages(
            key,
            [message(key, 1, "Late pre-delete response")],
            Some(301)
        )),
        false
    ));
    let row = &cache.get(key).unwrap().messages.as_slice()[0];
    assert!(row.metadata.deleted);
    assert!(row.body.as_str().is_empty());
}

#[test]
fn background_poll_does_not_settle_or_erase_an_older_page_failure() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    complete(
        &mut cache,
        key,
        1,
        Work::History,
        messages(key, [message(key, 50, "Loaded")], Some(50)),
    );
    let older = admit(&mut cache, key, 2, Work::Older);
    complete(&mut cache, key, 3, Work::Poll, messages(key, [], Some(70)));
    assert_eq!(
        cache.get(key).unwrap().state,
        RemoteState::Loading(RequestId(2))
    );
    let failed = Completion::Failed(UnavailableReason::Unconfigured);
    assert!(cache.complete(key, older, Work::Older, RequestId(2), &Err(failed), false));
    complete(&mut cache, key, 4, Work::Poll, messages(key, [], Some(71)));
    assert_eq!(
        cache.get(key).unwrap().state,
        RemoteState::Failed(RequestId(2), UnavailableReason::Unconfigured)
    );
    assert_eq!(cache.get(key).unwrap().older, Some(50));
}

#[test]
fn denial_and_logout_retire_tickets_without_changing_other_accounts() {
    let mut model = super::super::tests::model();
    let a = context(1, 1, 1, 1);
    let b = context(2, 1, 1, 1);
    select(&mut model, a);
    complete(
        &mut model.history,
        a,
        1,
        Work::History,
        messages(a, [message(a, 1, "Private A")], None),
    );
    let old = admit(&mut model.history, a, 2, Work::History);
    let deny = admit(&mut model.history, a, 3, Work::Poll);
    select(&mut model, b);
    complete(
        &mut model.history,
        b,
        4,
        Work::History,
        messages(b, [message(b, 1, "Private B")], None),
    );
    model.project_history(b);
    model.apply_message(
        RequestId(3),
        a,
        deny,
        Work::Poll,
        Err(IdentityError::Request(ErrorKind::PermissionDenied)),
    );
    assert!(model.history.get(a).is_none());
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Private B"
    );
    model.history.open(a);
    model.apply_message(
        RequestId(2),
        a,
        old,
        Work::History,
        reply(
            a,
            old,
            messages(a, [message(a, 1, "Obsolete secret")], None),
        ),
    );
    assert!(model.history.get(a).unwrap().messages.as_slice().is_empty());
    let after_logout = admit(&mut model.history, a, 5, Work::History);
    model.history.remove_account(a.account.account);
    model.apply_message(
        RequestId(5),
        a,
        after_logout,
        Work::History,
        reply(
            a,
            after_logout,
            messages(a, [message(a, 1, "Logged out secret")], None),
        ),
    );
    assert!(model.history.get(a).is_none());
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Private B"
    );
}

#[test]
fn obsolete_room_denial_cannot_erase_a_newer_successful_observation() {
    for error in [ErrorKind::PermissionDenied, ErrorKind::NotFound] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        let old = admit(&mut model.history, key, 1, Work::History);
        let current = admit(&mut model.history, key, 2, Work::Poll);
        model.apply_message(
            RequestId(2),
            key,
            current,
            Work::Poll,
            reply(
                key,
                current,
                messages(
                    key,
                    [message(key, 7, "Current authorized content")],
                    Some(7),
                ),
            ),
        );
        model.apply_message(
            RequestId(1),
            key,
            old,
            Work::History,
            Err(IdentityError::Request(error)),
        );
        assert_eq!(model.snapshot.content, ContentState::Ready);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Current authorized content"
        );
        assert_eq!(
            model.snapshot.message_panel.as_ref().unwrap().history,
            RemoteState::Idle
        );
        assert!(model.history.get(key).is_some());
    }
}

#[test]
fn denied_thread_or_message_context_does_not_revoke_conversation_history() {
    for work in [Work::Thread, Work::Context(9)] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Readable conversation")], Some(7)),
        );
        model.project_history(key);
        let operation = admit_target(&mut model, key, 2, work);
        model.apply_message(
            RequestId(2),
            key,
            operation,
            work,
            Err(IdentityError::Request(ErrorKind::PermissionDenied)),
        );
        assert_eq!(model.snapshot.content, ContentState::Ready);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Readable conversation"
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert_eq!(panel.history, RemoteState::Idle);
        assert_eq!(panel.state, RemoteState::Idle);
        let state = if work == Work::Thread {
            panel.thread_state
        } else {
            panel.context_load.as_ref().unwrap().state
        };
        assert_eq!(
            state,
            RemoteState::Failed(RequestId(2), UnavailableReason::PermissionDenied)
        );
    }
}

#[test]
fn target_failure_then_successful_retry_clears_only_its_own_error() {
    for work in [Work::Thread, Work::Context(9)] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Retained conversation")], Some(7)),
        );
        model.project_history(key);
        let failure = admit_target(&mut model, key, 2, work);
        model.apply_message(
            RequestId(2),
            key,
            failure,
            work,
            Err(IdentityError::Request(ErrorKind::ServerUnavailable)),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        let state = if work == Work::Thread {
            panel.thread_state
        } else {
            panel.context_load.as_ref().unwrap().state
        };
        assert!(matches!(state, RemoteState::Failed(RequestId(2), _)));
        assert_eq!(panel.state, RemoteState::Idle);
        assert!(panel.result.is_none());
        assert_eq!(
            model.snapshot.content,
            ContentState::Ready,
            "target endpoint failure cannot change room availability"
        );
        let retry = admit_target(&mut model, key, 3, work);
        model.apply_message(
            RequestId(3),
            key,
            retry,
            work,
            reply(
                key,
                retry,
                messages(key, [message(key, 9, "Located reply")], None),
            ),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        let state = if work == Work::Thread {
            panel.thread_state
        } else {
            panel.context_load.as_ref().unwrap().state
        };
        assert_eq!(state, RemoteState::Idle);
        assert_eq!(
            panel.state,
            RemoteState::Idle,
            "successful retry leaves no shared chat error"
        );
        assert!(panel.result.is_none());
        if matches!(work, Work::Context(_)) {
            assert_eq!(panel.located, Some((RequestId(3), 9)));
        }
        assert!(
            model
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|m| m.local_id == 7 && m.body.as_str() == "Retained conversation")
        );
    }
}

#[test]
fn target_denial_and_retry_cannot_settle_or_unlock_a_concurrent_mutation() {
    for work in [Work::Thread, Work::Context(9)] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Retained conversation")], Some(7)),
        );
        model.project_history(key);
        let mutation = admit(&mut model.history, key, 2, Work::Action);
        model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(2));
        let target = admit_target(&mut model, key, 3, work);
        model.apply_message(
            RequestId(3),
            key,
            target,
            work,
            Err(IdentityError::Request(ErrorKind::PermissionDenied)),
        );
        let retry = admit_target(&mut model, key, 4, work);
        model.apply_message(
            RequestId(4),
            key,
            retry,
            work,
            reply(
                key,
                retry,
                messages(key, [message(key, 9, "Target reply")], None),
            ),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert_eq!(panel.state, RemoteState::Saving(RequestId(2)));
        assert!(panel.result.is_none());
        assert_eq!(model.snapshot.content, ContentState::Ready);
        assert_eq!(
            model.message_action(
                RequestId(5),
                key,
                talk_core::ui::messaging::MessageAction::Edit {
                    message: 7,
                    body: text("Second edit")
                }
            ),
            Err(SubmitError::Busy),
            "the outstanding mutation still owns admission"
        );
        model.apply_message(
            RequestId(2),
            key,
            mutation,
            Work::Action,
            reply(
                key,
                mutation,
                MessagingResult::Message(Box::new(message(key, 7, "Confirmed edit"))),
            ),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert_eq!(panel.state, RemoteState::Idle);
        assert_eq!(
            panel.result,
            Some((RequestId(2), Completion::ServerAccepted))
        );
        assert!(
            model
                .snapshot
                .messages
                .as_slice()
                .iter()
                .any(|m| m.local_id == 7 && m.body.as_str() == "Confirmed edit")
        );
    }
}

#[test]
fn obsolete_context_target_completions_retire_without_touching_a_new_panel() {
    let mut model = super::super::tests::model();
    let key = context(1, 1, 1, 1);
    select(&mut model, key);
    complete(
        &mut model.history,
        key,
        1,
        Work::History,
        messages(key, [message(key, 7, "Current history")], Some(7)),
    );
    model.project_history(key);
    let old = admit_target(&mut model, key, 2, Work::Context(9));
    // Reopening the same room creates a different panel owner in the same
    // account runtime; matching the room alone is insufficient correlation.
    model.snapshot.message_panel = Some(MessagePanel::new(key));
    let current = admit_target(&mut model, key, 3, Work::Context(10));
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(99));
    model.apply_message(
        RequestId(2),
        key,
        old,
        Work::Context(9),
        reply(
            key,
            old,
            messages(key, [message(key, 9, "Obsolete target")], None),
        ),
    );
    assert!(
        !model
            .history
            .get(key)
            .unwrap()
            .messages
            .as_slice()
            .iter()
            .any(|m| m.local_id == 9)
    );
    let late_denial = admit(&mut model.history, key, 4, Work::Context(9));
    model.apply_message(
        RequestId(4),
        key,
        late_denial,
        Work::Context(9),
        Err(IdentityError::Request(ErrorKind::PermissionDenied)),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.context_load.as_ref().unwrap().target, 10);
    assert_eq!(
        panel.context_load.as_ref().unwrap().state,
        RemoteState::Loading(RequestId(3))
    );
    assert_eq!(panel.state, RemoteState::Saving(RequestId(99)));
    assert!(panel.located.is_none());
    assert_eq!(model.snapshot.content, ContentState::Ready);
    model.apply_message(
        RequestId(3),
        key,
        current,
        Work::Context(10),
        reply(
            key,
            current,
            messages(key, [message(key, 10, "Current target")], None),
        ),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(
        panel.context_load.as_ref().unwrap().state,
        RemoteState::Idle
    );
    assert_eq!(panel.located, Some((RequestId(3), 10)));
    assert_eq!(panel.state, RemoteState::Saving(RequestId(99)));
    assert!(
        model.history.flights.is_empty(),
        "obsolete target tickets must be retired"
    );
}

#[test]
fn target_retry_does_not_clear_an_unrelated_settled_mutation_error() {
    let mut model = super::super::tests::model();
    let key = context(1, 1, 1, 1);
    select(&mut model, key);
    let mutation = admit(&mut model.history, key, 1, Work::Action);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(1));
    model.apply_message(
        RequestId(1),
        key,
        mutation,
        Work::Action,
        Err(IdentityError::Request(ErrorKind::PermissionDenied)),
    );
    let before = model.snapshot.message_panel.as_ref().unwrap().state;
    let receipt = model.snapshot.message_panel.as_ref().unwrap().result;
    let context_request = admit_target(&mut model, key, 2, Work::Context(9));
    model.apply_message(
        RequestId(2),
        key,
        context_request,
        Work::Context(9),
        reply(
            key,
            context_request,
            messages(key, [message(key, 9, "Read target")], None),
        ),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.state, before);
    assert_eq!(panel.result, receipt);
    assert_eq!(
        panel.context_load.as_ref().unwrap().state,
        RemoteState::Idle
    );
}

#[test]
fn missing_context_target_is_a_scoped_failure_instead_of_successful_navigation() {
    for result in [
        messages(context(1, 1, 1, 1), [], None),
        MessagingResult::Changed,
    ] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Readable conversation")], Some(7)),
        );
        model.project_history(key);
        let operation = admit_target(&mut model, key, 2, Work::Context(9));
        model.apply_message(
            RequestId(2),
            key,
            operation,
            Work::Context(9),
            reply(key, operation, result),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert!(matches!(
            panel.context_load.as_ref().unwrap().state,
            RemoteState::Failed(RequestId(2), _)
        ));
        assert!(panel.located.is_none());
        assert_eq!(panel.state, RemoteState::Idle);
        assert_eq!(model.snapshot.content, ContentState::Ready);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Readable conversation"
        );
    }
}

#[test]
fn thread_completion_settles_only_its_current_request_and_retains_retry_content() {
    let mut model = super::super::tests::model();
    let key = context(1, 1, 1, 1);
    select(&mut model, key);
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    panel.thread_root = Some(7);
    panel.thread_state = RemoteState::Loading(RequestId(1));
    let operation = admit(&mut model.history, key, 1, Work::Thread);
    model.apply_message(
        RequestId(1),
        key,
        operation,
        Work::Thread,
        reply(
            key,
            operation,
            messages(key, [message(key, 8, "Thread reply")], None),
        ),
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().thread_state,
        RemoteState::Idle
    );
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .thread_messages
            .as_slice()
            .len(),
        1
    );
    model.snapshot.message_panel.as_mut().unwrap().thread_state =
        RemoteState::Loading(RequestId(2));
    let failure = admit(&mut model.history, key, 2, Work::Thread);
    model.apply_message(
        RequestId(2),
        key,
        failure,
        Work::Thread,
        Err(IdentityError::Request(ErrorKind::ServerUnavailable)),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert!(matches!(
        panel.thread_state,
        RemoteState::Failed(RequestId(2), _)
    ));
    assert_eq!(panel.thread_messages.as_slice().len(), 1);
    let stale = admit(&mut model.history, key, 3, Work::Thread);
    model.snapshot.message_panel.as_mut().unwrap().thread_state =
        RemoteState::Loading(RequestId(4));
    model.apply_message(
        RequestId(3),
        key,
        stale,
        Work::Thread,
        Err(IdentityError::Request(ErrorKind::PermissionDenied)),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.thread_state, RemoteState::Loading(RequestId(4)));
    assert_eq!(panel.thread_messages.as_slice().len(), 1);
    let deny = admit(&mut model.history, key, 4, Work::Thread);
    model.apply_message(
        RequestId(4),
        key,
        deny,
        Work::Thread,
        Err(IdentityError::Request(ErrorKind::PermissionDenied)),
    );
    assert!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .thread_messages
            .as_slice()
            .is_empty()
    );
}

#[test]
fn selected_room_retry_preserves_loaded_panel_and_reopens_a_denied_read() {
    let mut model = super::super::tests::model();
    let key = context(1, 1, 1, 1);
    select(&mut model, key);
    complete(
        &mut model.history,
        key,
        1,
        Work::History,
        messages(key, [message(key, 7, "Retained content")], Some(7)),
    );
    model.project_history(key);
    model.snapshot.message_panel.as_mut().unwrap().thread_root = Some(7);
    // This state fixture has no worker: admission failure must still settle the
    // retry, retain loaded content and preserve the current panel's local scope.
    assert!(model.refresh_messages(key).is_err());
    assert_eq!(model.snapshot.content, ContentState::Ready);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "Retained content"
    );
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().thread_root,
        Some(7)
    );
    model.invalidate_history(key, ContentState::Denied);
    assert!(model.refresh_messages(key).is_err());
    assert_eq!(model.snapshot.content, ContentState::Failed);
    assert!(model.snapshot.messages.as_slice().is_empty());
    assert!(model.history.get(key).is_some());
}

#[test]
fn mismatched_payload_scope_cannot_enter_an_admitted_room() {
    let mut model = super::super::tests::model();
    let a = context(1, 1, 1, 1);
    let b = context(2, 1, 1, 1);
    select(&mut model, a);
    let operation = admit(&mut model.history, a, 1, Work::History);
    model.apply_message(
        RequestId(1),
        a,
        operation,
        Work::History,
        reply(
            a,
            operation,
            messages(b, [message(b, 1, "Foreign content")], Some(1)),
        ),
    );
    assert!(model.snapshot.messages.as_slice().is_empty());
    assert!(!model.history.get(a).unwrap().loaded);
    assert_eq!(model.snapshot.content, ContentState::Failed);
}

#[test]
fn evicting_history_does_not_drop_an_admitted_send_receipt_or_replay_it() {
    let mut model = super::super::tests::model();
    let a = context(1, 1, 1, 1);
    let b = context(2, 1, 1, 1);
    select(&mut model, a);
    let operation = admit(&mut model.history, a, 1, Work::Send);
    model.unresolved.push(Unresolved {
        request: RequestId(1),
        context: a,
        operation,
    });
    select(&mut model, b);
    model.history.remove(a);
    model.apply_message(
        RequestId(1),
        a,
        operation,
        Work::Send,
        reply(
            a,
            operation,
            MessagingResult::Send(protocol::SendReceipt {
                operation,
                state: SendState::ServerAccepted(MessageId {
                    conversation: a.conversation,
                    sequence: 3,
                }),
            }),
        ),
    );
    assert!(model.unresolved.is_empty());
    assert_eq!(model.snapshot.selected, Some(b));
    assert!(model.history.get(a).is_none());
    assert_eq!(
        model.snapshot.submission.unwrap().result,
        Completion::ServerAccepted
    );
    assert!(
        model.pending.is_empty(),
        "completion must not replay a send"
    );
}

#[test]
fn expired_content_and_parent_excerpts_are_removed_before_restoring_history() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    let mut parent = message(key, 1, "Expiring secret");
    parent.message.expires_at = Some(100);
    let mut child = message(key, 2, "Reply");
    child.parent = Some(ReplyContext {
        id: parent.message.id,
        author: text("User"),
        text: text("Expiring secret"),
        unavailable: false,
    });
    complete(
        &mut cache,
        key,
        1,
        Work::History,
        messages(key, [parent, child], None),
    );
    cache.expire(99);
    assert_eq!(cache.get(key).unwrap().messages.as_slice().len(), 2);
    cache.expire(100);
    let rows = cache.get(key).unwrap().messages.as_slice();
    assert_eq!(rows.len(), 1);
    let parent = rows[0].metadata.parent.as_ref().unwrap();
    assert!(parent.unavailable);
    assert!(parent.text.as_str().is_empty());
}

#[test]
fn room_capacity_uses_open_order_and_evicted_replies_cannot_recreate_content() {
    let mut cache = Cache::default();
    let first = context(1, 1, 1, 1);
    cache.open(first);
    let obsolete = admit(&mut cache, first, 1, Work::History);
    for room in 2..=ROOMS as u128 {
        cache.open(context(1, 1, 1, room));
    }
    // Completion order is not an LRU touch; deliberate reopening is.
    cache.open(context(1, 1, 1, 2));
    cache.open(context(1, 1, 1, ROOMS as u128 + 1));
    assert_eq!(cache.rooms.len(), ROOMS);
    assert!(cache.get(first).is_none());
    cache.open(first);
    assert!(!cache.complete(
        first,
        obsolete,
        Work::History,
        RequestId(1),
        &Ok(messages(first, [message(first, 1, "Evicted secret")], None)),
        false
    ));
    assert!(cache.get(first).unwrap().messages.as_slice().is_empty());
    assert_eq!(cache.rooms.len(), ROOMS);
}

#[test]
fn newer_connectivity_observations_keep_warm_rows_and_win_over_late_completions() {
    for error in [
        ErrorKind::Transport,
        ErrorKind::ServerAccessDenied,
        ErrorKind::Deadline,
        ErrorKind::ServerUnavailable,
    ] {
        let mut model = super::super::tests::model();
        let key = context(1, 1, 1, 1);
        select(&mut model, key);
        let initial = admit(&mut model.history, key, 1, Work::History);
        model.apply_message(
            RequestId(1),
            key,
            initial,
            Work::History,
            reply(
                key,
                initial,
                messages(key, [message(key, 1, "Retained")], Some(1)),
            ),
        );
        let late_failure = admit(&mut model.history, key, 2, Work::History);
        let failure = admit(&mut model.history, key, 3, Work::Poll);
        model.apply_message(
            RequestId(3),
            key,
            failure,
            Work::Poll,
            Err(IdentityError::Request(error)),
        );
        assert_eq!(model.snapshot.content, ContentState::Offline);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "Retained"
        );
        select(&mut model, key);
        assert_eq!(
            model.snapshot.content,
            ContentState::Offline,
            "warm navigation retains reachability"
        );
        let recovery = admit(&mut model.history, key, 4, Work::Poll);
        model.apply_message(
            RequestId(4),
            key,
            recovery,
            Work::Poll,
            reply(
                key,
                recovery,
                messages(key, [message(key, 2, "Recovered gap")], Some(2)),
            ),
        );
        assert_eq!(model.snapshot.content, ContentState::Ready);
        model.apply_message(
            RequestId(2),
            key,
            late_failure,
            Work::History,
            Err(IdentityError::Request(error)),
        );
        assert_eq!(
            model.snapshot.content,
            ContentState::Ready,
            "old failure cannot hide recovery"
        );
        let late_success = admit(&mut model.history, key, 5, Work::History);
        let failure = admit(&mut model.history, key, 6, Work::Poll);
        model.apply_message(
            RequestId(6),
            key,
            failure,
            Work::Poll,
            Err(IdentityError::Request(error)),
        );
        model.apply_message(
            RequestId(5),
            key,
            late_success,
            Work::History,
            reply(
                key,
                late_success,
                messages(key, [message(key, 1, "Retained")], Some(1)),
            ),
        );
        assert_eq!(
            model.snapshot.content,
            ContentState::Offline,
            "old success cannot hide a newer outage"
        );
        assert_eq!(model.snapshot.messages.as_slice().len(), 2);
    }
}

#[test]
fn prefetched_history_is_ready_on_open_and_retired_tickets_cannot_restore_it() {
    let room = context(1, 2, 3, 4);
    let mut model = super::super::tests::model();
    model.history.open(room);
    complete(
        &mut model.history,
        room,
        1,
        Work::Prefetch,
        messages(
            room,
            [message(room, 1, "Synthetic prefetched message")],
            Some(1),
        ),
    );
    assert!(model.snapshot.messages.as_slice().is_empty());
    assert!(model.snapshot.selected.is_none());
    select(&mut model, room);
    assert_eq!(model.snapshot.messages.as_slice().len(), 1);
    assert_eq!(model.snapshot.content, ContentState::Ready);
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().history,
        RemoteState::Idle
    );
    let operation = admit(&mut model.history, room, 2, Work::Prefetch);
    model.history.remove_account(room.account.account);
    assert!(!model.history.complete(
        room,
        operation,
        Work::Prefetch,
        RequestId(2),
        &Ok(messages(room, [message(room, 2, "Late")], Some(2))),
        false
    ));
    assert!(model.history.get(room).is_none());
}

#[test]
fn speculative_history_never_evicts_previously_opened_rooms_at_capacity() {
    let mut cache = Cache::default();
    for room in 1..=ROOMS {
        cache.open(context(1, 1, 1, room as u128));
    }
    let foreground = context(1, 1, 1, 1);
    let speculative = context(1, 1, 1, 100);
    assert!(!cache.prefetch(speculative));
    assert!(cache.get(foreground).is_some());
    assert!(cache.get(speculative).is_none());
    assert_eq!(cache.rooms.len(), ROOMS);
}

#[test]
fn opened_room_keeps_priority_when_new_navigation_reclaims_a_prefetch_slot() {
    let mut cache = Cache::default();
    let opened = context(1, 1, 1, 1);
    cache.open(opened);
    for room in 2..=ROOMS {
        assert!(cache.prefetch(context(1, 1, 1, room as u128)));
    }
    cache.open(context(1, 1, 1, 100));
    assert!(cache.get(opened).is_some());
    assert!(cache.get(context(1, 1, 1, 2)).is_none());
    assert_eq!(cache.rooms.len(), ROOMS);
}

fn with_capabilities(mut result: MessagingResult, allowed: Decision) -> MessagingResult {
    let MessagingResult::Messages { capabilities, .. } = &mut result else {
        panic!("fixture history")
    };
    *capabilities = Some(protocol::messaging::MessagingCapabilities {
        pin_change: allowed,
        actions: page([(
            protocol::messaging::MessagingFeature::React,
            protocol::conversation::ActionAvailability {
                implemented: true,
                server: protocol::FeatureSupport::Supported,
                allowed,
            },
        )]),
        message_limit: Some(2048),
        file_limit: 12_000,
        read_receipts: Some(true),
        typing_status: Some(true),
    });
    result
}

#[test]
fn prefetched_history_restores_recent_composer_capabilities_without_another_probe() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    assert!(cache.prefetch(key));
    complete(
        &mut cache,
        key,
        1,
        Work::Prefetch,
        with_capabilities(
            messages(key, [message(key, 1, "History")], None),
            Decision::Allowed,
        ),
    );
    cache.open(key);
    let mut panel = MessagePanel::new(key);
    cache.get(key).unwrap().project(&mut panel);
    assert_eq!(panel.message_limit, 2048);
    assert_eq!(panel.file_limit, 12_000);
    assert!(
        panel
            .features
            .as_slice()
            .iter()
            .any(
                |(feature, value)| *feature == talk_core::ui::messaging::MessageFeature::React
                    && value.allowed == Decision::Allowed
            )
    );
    assert!(cache.get(key).unwrap().loaded);
}

#[test]
fn confirmed_pins_fence_older_reads_and_reconcile_edits_deletes_and_reopens() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    complete(
        &mut cache,
        key,
        1,
        Work::History,
        messages(key, [message(key, 7, "Original")], None),
    );
    let old_list = admit(&mut cache, key, 2, Work::Pins);
    complete(
        &mut cache,
        key,
        3,
        Work::PinChange {
            message: 7,
            pinned: true,
        },
        MessagingResult::Changed,
    );
    assert!(cache.complete(
        key,
        old_list,
        Work::Pins,
        RequestId(2),
        &Ok(messages(key, [], None)),
        false
    ));
    let mut panel = MessagePanel::new(key);
    cache.get(key).unwrap().project(&mut panel);
    assert_eq!(panel.pinned_messages.as_slice()[0].local_id, 7);
    assert_eq!(panel.pins_state, RemoteState::Idle);
    complete(
        &mut cache,
        key,
        4,
        Work::Action,
        MessagingResult::Message(Box::new(message(key, 7, "Edited"))),
    );
    cache.get(key).unwrap().project(&mut panel);
    assert_eq!(panel.pinned_messages.as_slice()[0].body.as_str(), "Edited");
    let old_list = admit(&mut cache, key, 5, Work::Pins);
    complete(
        &mut cache,
        key,
        6,
        Work::PinChange {
            message: 7,
            pinned: false,
        },
        MessagingResult::Changed,
    );
    assert!(cache.complete(
        key,
        old_list,
        Work::Pins,
        RequestId(5),
        &Ok(messages(key, [message(key, 7, "Original")], None)),
        false
    ));
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.pinned_messages.as_slice().is_empty());
    complete(
        &mut cache,
        key,
        7,
        Work::PinChange {
            message: 7,
            pinned: true,
        },
        MessagingResult::Changed,
    );
    let mut deleted = message(key, 7, "");
    deleted.message.kind = MessageKind::Deleted;
    complete(
        &mut cache,
        key,
        8,
        Work::Action,
        MessagingResult::Message(Box::new(deleted)),
    );
    cache.open(key);
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.pinned_messages.as_slice().is_empty());
}

#[test]
fn pin_reads_and_failures_have_separate_state_and_never_invent_confirmed_membership() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    let read = admit(&mut cache, key, 1, Work::Pins);
    let mut panel = MessagePanel::new(key);
    cache.get(key).unwrap().project(&mut panel);
    assert_eq!(panel.pins_state, RemoteState::Loading(RequestId(1)));
    assert!(!panel.pins_loaded);
    assert_eq!(panel.state, RemoteState::Idle);
    assert!(cache.complete(
        key,
        read,
        Work::Pins,
        RequestId(1),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied)),
        false
    ));
    cache.get(key).unwrap().project(&mut panel);
    assert_eq!(
        panel.pins_state,
        RemoteState::Failed(RequestId(1), UnavailableReason::PermissionDenied)
    );
    assert!(!panel.pins_loaded);
    complete(&mut cache, key, 2, Work::Pins, messages(key, [], None));
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.pins_loaded);
    let change = Work::PinChange {
        message: 7,
        pinned: true,
    };
    let operation = admit(&mut cache, key, 3, change);
    assert!(cache.complete(
        key,
        operation,
        change,
        RequestId(3),
        &Err(Completion::Uncertain),
        false
    ));
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.pinned_messages.as_slice().is_empty());
    let obsolete = admit(&mut cache, key, 4, Work::Pins);
    cache.remove(key);
    cache.open(key);
    assert!(!cache.complete(
        key,
        obsolete,
        Work::Pins,
        RequestId(4),
        &Ok(messages(key, [message(key, 7, "Retired")], None)),
        false
    ));
}

#[test]
fn forwarding_confirms_only_its_named_destination_without_inserting_into_source() {
    let source = context(1, 1, 1, 1);
    let destination = context(1, 1, 1, 2);
    for (returned, expected) in [
        (destination, Completion::ServerAccepted),
        (source, Completion::Uncertain),
    ] {
        let mut model = super::super::tests::model();
        select(&mut model, source);
        complete(
            &mut model.history,
            source,
            1,
            Work::History,
            messages(source, [message(source, 7, "Source")], None),
        );
        let work = Work::Forward(destination);
        let operation = admit(&mut model.history, source, 2, work);
        model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(2));
        model.apply_message(
            RequestId(2),
            source,
            operation,
            work,
            reply(
                source,
                operation,
                MessagingResult::Send(protocol::SendReceipt {
                    operation,
                    state: protocol::SendState::ServerAccepted(MessageId {
                        conversation: returned.conversation,
                        sequence: 99,
                    }),
                }),
            ),
        );
        assert_eq!(
            model.snapshot.message_panel.as_ref().unwrap().result,
            Some((RequestId(2), expected))
        );
        assert_eq!(model.snapshot.messages.as_slice().len(), 1);
        assert_eq!(model.snapshot.messages.as_slice()[0].local_id, 7);
    }
}

#[test]
fn reminder_changes_and_schedule_cancellation_preserve_other_confirmed_entries() {
    let key = context(1, 1, 1, 1);
    let mut model = super::super::tests::model();
    select(&mut model, key);
    model.snapshot.message_panel.as_mut().unwrap().reminders = page([
        talk_core::ui::messaging::Reminder {
            message: 7,
            timestamp: 100,
        },
        talk_core::ui::messaging::Reminder {
            message: 8,
            timestamp: 200,
        },
    ]);
    let work = Work::Reminder(7);
    let operation = admit(&mut model.history, key, 1, work);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(1));
    model.apply_message(
        RequestId(1),
        key,
        operation,
        work,
        reply(
            key,
            operation,
            MessagingResult::Reminder(Some(protocol::messaging::MessageReminder {
                message: MessageId {
                    conversation: key.conversation,
                    sequence: 7,
                },
                timestamp: 300,
            })),
        ),
    );
    let reminders = &model.snapshot.message_panel.as_ref().unwrap().reminders;
    assert_eq!(reminders.as_slice().len(), 2);
    assert!(
        reminders
            .as_slice()
            .iter()
            .any(|r| r.message == 7 && r.timestamp == 300)
    );
    let operation = admit(&mut model.history, key, 2, work);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(2));
    model.apply_message(
        RequestId(2),
        key,
        operation,
        work,
        reply(key, operation, MessagingResult::Reminder(None)),
    );
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .reminders
            .as_slice(),
        &[talk_core::ui::messaging::Reminder {
            message: 8,
            timestamp: 200
        }]
    );
    model.snapshot.message_panel.as_mut().unwrap().scheduled =
        page([7, 8].map(|id| talk_core::ui::messaging::Scheduled {
            id,
            body: text("Scheduled"),
            timestamp: 100,
        }));
    let work = Work::CancelScheduled(7);
    let operation = admit(&mut model.history, key, 3, work);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(3));
    model.apply_message(
        RequestId(3),
        key,
        operation,
        work,
        reply(key, operation, MessagingResult::Changed),
    );
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .scheduled
            .as_slice()
            .len(),
        1
    );
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .scheduled
            .as_slice()[0]
            .id,
        8
    );
}

#[test]
fn older_reads_cannot_replace_newer_composer_capabilities_and_expired_sources_stay_unknown() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    let first = admit(&mut cache, key, 1, Work::History);
    let second = admit(&mut cache, key, 2, Work::Poll);
    assert!(cache.complete(
        key,
        second,
        Work::Poll,
        RequestId(2),
        &Ok(with_capabilities(messages(key, [], None), Decision::Denied)),
        false
    ));
    assert!(cache.complete(
        key,
        first,
        Work::History,
        RequestId(1),
        &Ok(with_capabilities(
            messages(key, [], None),
            Decision::Allowed
        )),
        false
    ));
    let mut panel = MessagePanel::new(key);
    cache.get(key).unwrap().project(&mut panel);
    assert!(
        panel
            .features
            .as_slice()
            .iter()
            .any(|(_, value)| value.allowed == Decision::Denied)
    );
    // Freshness starts at admission, so a slow response cannot create a fresh grant.
    let operation = admit(&mut cache, key, 3, Work::Poll);
    cache.flights.last_mut().unwrap().started = Instant::now() - Duration::from_secs(13);
    assert!(cache.complete(
        key,
        operation,
        Work::Poll,
        RequestId(3),
        &Ok(with_capabilities(
            messages(key, [], None),
            Decision::Allowed
        )),
        false
    ));
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.features.as_slice().is_empty());
    assert_eq!(panel.file_limit, 0);
    cache.remove(key);
    cache.open(key);
    cache.get(key).unwrap().project(&mut panel);
    assert!(panel.features.as_slice().is_empty());
}

fn reaction_result(key: ConversationRef, target: u128, count: u32) -> MessagingResult {
    MessagingResult::Reactions {
        message: MessageId {
            conversation: key.conversation,
            sequence: target.try_into().unwrap(),
        },
        reactions: if count == 0 {
            Page::empty()
        } else {
            page([talk_core::protocol::messaging::Reaction {
                emoji: text("👍"),
                count,
                own: true,
            }])
        },
    }
}

fn admit_reaction(
    model: &mut Accounts,
    key: ConversationRef,
    serial: u64,
    target: u128,
) -> OperationId {
    assert!(model.history.can_react(key, target));
    let operation = admit(&mut model.history, key, serial, Work::Reaction(target));
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    panel.state = RemoteState::Saving(RequestId(serial));
    panel.reaction_pending = Some(target);
    operation
}

#[test]
fn accepted_reactions_update_every_copy_immediately_and_older_reads_cannot_revert_them() {
    for target in [7, 90] {
        let key = context(1, 1, 1, 1);
        let mut model = super::super::tests::model();
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Main")], None),
        );
        model.project_history(key);
        let related = message_view(&message(key, target, "Related"));
        complete(
            &mut model.history,
            key,
            2,
            Work::Pins,
            messages(key, [message(key, target, "Related")], None),
        );
        let panel = model.snapshot.message_panel.as_mut().unwrap();
        panel.thread_root = Some(7);
        panel.thread_state = RemoteState::Loading(RequestId(4));
        panel.thread_messages = page([related.clone()]);
        panel.shared_files = page([related]);
        let poll = admit(&mut model.history, key, 3, Work::Poll);
        let thread = admit(&mut model.history, key, 4, Work::Thread);
        let mutation = admit_reaction(&mut model, key, 5, u128::from(target));
        model.apply_message(
            RequestId(5),
            key,
            mutation,
            Work::Reaction(u128::from(target)),
            reply(key, mutation, reaction_result(key, u128::from(target), 3)),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        for copy in [
            &panel.thread_messages,
            &panel.pinned_messages,
            &panel.shared_files,
        ] {
            assert_eq!(copy.as_slice()[0].metadata.reactions.as_slice()[0].count, 3);
        }
        assert_eq!(panel.reaction_pending, None);
        assert_eq!(panel.state, RemoteState::Idle);
        assert_eq!(
            panel.result,
            Some((RequestId(5), Completion::ServerAccepted))
        );
        assert_eq!(
            model.history.flights.len(),
            2,
            "accepted response starts no follow-up GET"
        );
        model.apply_message(
            RequestId(3),
            key,
            poll,
            Work::Poll,
            reply(key, poll, messages(key, [message(key, 7, "Main")], None)),
        );
        if target == 7 {
            assert_eq!(
                model.snapshot.messages.as_slice()[0]
                    .metadata
                    .reactions
                    .as_slice()[0]
                    .count,
                3
            );
        }
        model.apply_message(
            RequestId(4),
            key,
            thread,
            Work::Thread,
            reply(
                key,
                thread,
                messages(key, [message(key, target, "Related")], None),
            ),
        );
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert_eq!(
            panel.thread_messages.as_slice()[0]
                .metadata
                .reactions
                .as_slice()[0]
                .count,
            3
        );
        assert!(
            model
                .history
                .get(key)
                .unwrap()
                .reaction_snapshots
                .is_empty()
        );
    }
}

#[test]
fn reaction_failure_uncertainty_and_wrong_target_preserve_counts_and_settle_only_exact_request() {
    for failure in [0, 1, 2] {
        let key = context(1, 1, 1, 1);
        let mut model = super::super::tests::model();
        select(&mut model, key);
        complete(
            &mut model.history,
            key,
            1,
            Work::History,
            messages(key, [message(key, 7, "Main")], None),
        );
        model.project_history(key);
        let operation = admit_reaction(&mut model, key, 2, 7);
        // An unrelated UI request cannot consume this operation's admission or pending marker.
        model.apply_message(
            RequestId(99),
            key,
            operation,
            Work::Reaction(7),
            reply(key, operation, reaction_result(key, 7, 4)),
        );
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .reaction_pending,
            Some(7)
        );
        assert_eq!(model.history.flights.len(), 1);
        let result = match failure {
            0 => Err(IdentityError::Request(ErrorKind::PermissionDenied)),
            1 => Err(IdentityError::Request(ErrorKind::Uncertain)),
            _ => reply(key, operation, reaction_result(key, 8, 4)),
        };
        model.apply_message(RequestId(2), key, operation, Work::Reaction(7), result);
        let panel = model.snapshot.message_panel.as_ref().unwrap();
        assert_eq!(panel.reaction_pending, None);
        assert!(
            model.snapshot.messages.as_slice()[0]
                .metadata
                .reactions
                .as_slice()
                .is_empty()
        );
        assert_eq!(
            panel.state,
            if failure == 0 {
                RemoteState::Failed(RequestId(2), UnavailableReason::PermissionDenied)
            } else {
                RemoteState::Uncertain(RequestId(2))
            }
        );
    }
}

#[test]
fn reaction_completion_cannot_clear_a_newer_pending_mutation_or_recreate_retired_history() {
    let key = context(1, 1, 1, 1);
    let mut model = super::super::tests::model();
    select(&mut model, key);
    let old = admit_reaction(&mut model, key, 1, 7);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(2));
    model
        .snapshot
        .message_panel
        .as_mut()
        .unwrap()
        .reaction_pending = Some(8);
    model.apply_message(
        RequestId(1),
        key,
        old,
        Work::Reaction(7),
        reply(key, old, reaction_result(key, 7, 2)),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.state, RemoteState::Saving(RequestId(2)));
    assert_eq!(panel.reaction_pending, Some(8));
    let retired = admit(&mut model.history, key, 3, Work::Reaction(8));
    model.history.remove(key);
    model.apply_message(
        RequestId(3),
        key,
        retired,
        Work::Reaction(8),
        reply(key, retired, reaction_result(key, 8, 2)),
    );
    assert!(model.history.get(key).is_none());
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .reaction_pending,
        Some(8)
    );
}

#[test]
fn reaction_old_read_protection_is_bounded_and_reclaims_after_reads_settle() {
    let key = context(1, 1, 1, 1);
    let mut cache = Cache::default();
    cache.open(key);
    let old_read = admit(&mut cache, key, 1, Work::Poll);
    for target in 1..=REACTION_SNAPSHOTS {
        assert!(cache.can_react(key, target as u128));
        complete(
            &mut cache,
            key,
            target as u64 + 1,
            Work::Reaction(target as u128),
            reaction_result(key, target as u128, 1),
        );
    }
    assert!(!cache.can_react(key, 101));
    assert!(cache.can_react(key, 1));
    assert_eq!(
        cache.get(key).unwrap().reaction_snapshots.len(),
        REACTION_SNAPSHOTS
    );
    assert!(cache.complete(
        key,
        old_read,
        Work::Poll,
        RequestId(1),
        &Ok(messages(key, [], None)),
        false
    ));
    assert!(cache.get(key).unwrap().reaction_snapshots.is_empty());
    assert!(cache.can_react(key, 101));
    cache.remove_account(key.account.account);
    assert!(!cache.can_react(key, 1));
}
