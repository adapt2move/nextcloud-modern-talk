use super::*;
use talk_core::protocol::{Message, MessageId, MessageKind};

fn key(id: u128) -> NavigationRoomKey {
    NavigationRoomKey::new(&format!("r1:{id:032x}")).unwrap()
}
fn stored(
    operation: u128,
    room: NavigationRoomKey,
    intent: Intent,
    phase: OutboxPhase,
    ordinal: u64,
) -> OutboxEntry {
    let mut record =
        OutboxRecord::from_intent(OperationId::from_local(operation), room, &intent).unwrap();
    record.phase = phase;
    OutboxEntry {
        ordinal,
        record,
        saved_at: history::now_unix(),
    }
}
fn details(context: ConversationRef, id: u64, body: &str) -> wire::MessageDetails {
    wire::MessageDetails {
        message: Message {
            id: MessageId {
                conversation: context.conversation,
                sequence: id,
            },
            author: text("Synthetic actor"),
            text: text(body),
            kind: MessageKind::Comment,
            timestamp: 1,
            expires_at: None,
            edited_at: None,
            own_operation: None,
        },
        author_key: None,
        own: true,
        replyable: Some(true),
        parent: None,
        thread: None,
        reactions: Page::empty(),
        rich: Page::empty(),
        system: None,
        notify: Decision::Denied,
        edit: Decision::Allowed,
        delete: Decision::Allowed,
    }
}
fn observe(
    model: &mut Accounts,
    context: ConversationRef,
    serial: u64,
    rows: impl IntoIterator<Item = wire::MessageDetails>,
) {
    let operation = OperationId::from_local(1000 + u128::from(serial));
    model.history.admitted(
        context,
        operation,
        messaging::Work::History,
        RequestId(serial),
        serial,
    );
    assert!(model.history.complete(
        context,
        operation,
        messaging::Work::History,
        RequestId(serial),
        &Ok(wire::MessagingResult::Messages {
            capabilities: None,
            messages: page(rows),
            cursor: None,
            common_read: None,
        }),
        false
    ));
}
fn model() -> (Accounts, ConversationRef, NavigationRoomKey) {
    let mut model = crate::composition::accounts::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
    let context = model.snapshot.selected.unwrap();
    let room = key(context.conversation.local);
    model.snapshot.conversations = page(
        model
            .snapshot
            .conversations
            .as_slice()
            .iter()
            .cloned()
            .map(|mut row| {
                if row.context == context {
                    row.navigation_key = Some(room.clone());
                }
                row
            }),
    );
    model.snapshot.messages = Page::empty();
    model.snapshot.message_panel = Some(view::MessagePanel::new(context));
    model.history.open(context);
    (model, context, room)
}

#[test]
fn cached_active_scope_without_a_live_room_host_cannot_resolve_for_dispatch_or_marking() {
    use std::cell::Cell;
    use talk_core::ids::{HostId, RuntimeGeneration};
    let (mut model, live, room) = model();
    model.background.sync(&[live.account], Instant::now());
    let row = model
        .snapshot
        .conversations
        .as_slice()
        .iter()
        .find(|r| r.context == live)
        .unwrap()
        .clone();
    let mut cached = row.clone();
    cached.context.conversation.host = HostId::from_local(0);
    let checked = Cell::new(0);
    let current = |scope| {
        checked.set(checked.get() + 1);
        scope == live.account
    };
    model.background.account_mut(live.account).unwrap().rooms = page([cached.clone()]);
    // This is exactly the dangerous intermediate state: the account is active,
    // but its retained cache row still has no protocol-owned host identity.
    assert_eq!(
        model.outbox_context_with(live.account.account, &room, current),
        None
    );
    assert_eq!(
        checked.get(),
        0,
        "presentation identities never reach authority validation"
    );
    let mut local_runtime = row.clone();
    local_runtime.context.account.runtime = RuntimeGeneration::from_local(0);
    model.background.account_mut(live.account).unwrap().rooms = page([local_runtime]);
    assert_eq!(
        model.outbox_context_with(live.account.account, &room, current),
        None
    );
    assert_eq!(checked.get(), 0);
    // A fresh row becomes usable only after its independently validated current
    // scope. A leftover synthetic row may not hide that fresh publication.
    model.background.account_mut(live.account).unwrap().rooms = page([cached, row]);
    assert_eq!(
        model.outbox_context_with(live.account.account, &room, current),
        Some(live)
    );
    assert_eq!(checked.get(), 1);
    assert_eq!(
        model.outbox_context_with(live.account.account, &room, |_| false),
        None
    );
}

#[test]
fn canonical_projection_keeps_pending_overlays_and_failed_edit_reveals_latest_server_text() {
    let (mut model, context, room) = model();
    observe(&mut model, context, 1, [details(context, 7, "original")]);
    model.outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                room.clone(),
                Intent::Edit {
                    message: 7,
                    body: text("local edit"),
                },
                OutboxPhase::Queued,
                1,
            ),
            stored(
                2,
                room.clone(),
                Intent::Reaction {
                    message: 7,
                    emoji: text("👍"),
                    add: true,
                },
                OutboxPhase::Queued,
                2,
            ),
        ]),
    );
    model.project_history(context);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "local edit"
    );
    assert!(
        model.snapshot.messages.as_slice()[0]
            .metadata
            .reactions
            .as_slice()
            .iter()
            .any(|r| r.emoji.as_str() == "👍" && r.own && r.count == 1)
    );
    assert_eq!(
        model.history.get(context).unwrap().messages.as_slice()[0]
            .body
            .as_str(),
        "original"
    );

    observe(
        &mut model,
        context,
        2,
        [details(context, 7, "new canonical from another client")],
    );
    model.project_history(context);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "local edit"
    );
    assert!(model.outbox_result(
        OperationId::from_local(1),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied))
    ));
    model.project_history(context);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "new canonical from another client"
    );
    assert!(
        model.snapshot.messages.as_slice()[0]
            .metadata
            .reactions
            .as_slice()
            .iter()
            .any(|r| r.own)
    );
    assert!(
        !model.history.get(context).unwrap().messages.as_slice()[0]
            .metadata
            .reactions
            .as_slice()
            .iter()
            .any(|r| r.own)
    );
    assert_eq!(
        model
            .snapshot
            .message_panel
            .as_ref()
            .unwrap()
            .action_error
            .unwrap()
            .1,
        view::MessageActionError::Failed(UnavailableReason::PermissionDenied)
    );
}

#[test]
fn restart_restores_order_and_never_replays_a_dispatched_mutation_as_a_new_write() {
    let context = crate::composition::fixtures::context(1);
    let room = key(1);
    let first = stored(
        11,
        room.clone(),
        Intent::Delete { message: 7 },
        OutboxPhase::Dispatched,
        1,
    );
    let second = stored(
        12,
        room.clone(),
        Intent::Reaction {
            message: 8,
            emoji: text("👍"),
            add: true,
        },
        OutboxPhase::Queued,
        2,
    );
    let third = stored(
        13,
        room,
        Intent::Send {
            body: text("already possibly sent"),
            options: Default::default(),
        },
        OutboxPhase::Uncertain,
        3,
    );
    let mut outbox = Outbox::default();
    let loaded = page([third, second, first]);
    outbox.load(context.account.account, &loaded);
    outbox.load(context.account.account, &loaded);
    assert_eq!(outbox.entries.len(), 3);
    assert_eq!(
        outbox.entries.iter().map(|e| e.ordinal).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert!(outbox.entries[0].state == State::Uncertain);
    assert_eq!(outbox.entries[0].record.phase, OutboxPhase::Uncertain);
    assert!(outbox.entries[1].state == State::Queued);
    assert!(outbox.entries[2].state == State::Uncertain);
    for entry in outbox
        .entries
        .iter()
        .filter(|e| e.state == State::Uncertain)
    {
        assert!(matches!(
            reconcile_command(context, &entry.intent).unwrap().0,
            wire::MessagingCommand::Context(_) | wire::MessagingCommand::ReconcileSend
        ));
    }
}

#[test]
fn a_late_failure_removes_only_its_own_overlay_and_keeps_a_newer_local_edit() {
    let (mut model, context, room) = model();
    observe(&mut model, context, 1, [details(context, 7, "original")]);
    model.outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                room.clone(),
                Intent::Edit {
                    message: 7,
                    body: text("first local edit"),
                },
                OutboxPhase::Queued,
                1,
            ),
            stored(
                2,
                room,
                Intent::Edit {
                    message: 7,
                    body: text("second local edit"),
                },
                OutboxPhase::Queued,
                2,
            ),
        ]),
    );
    model.project_history(context);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "second local edit"
    );
    model.outbox_result(
        OperationId::from_local(1),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied)),
    );
    model.project_history(context);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "second local edit"
    );
    assert!(!model.outbox_result(
        OperationId::from_local(999),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied))
    ));
    assert!(
        model
            .outbox
            .entries
            .iter()
            .any(|e| e.record.operation == OperationId::from_local(2) && e.state != State::Failed)
    );
}

#[test]
fn pending_bubbles_are_bounded_without_evicting_canonical_history_and_projection_is_idempotent() {
    let (mut model, context, room) = model();
    // Network pages are bounded to 50, so build the canonical 100 rows through
    // two real admitted history completions, as production pagination does.
    observe(
        &mut model,
        context,
        1,
        (1..=50).map(|id| details(context, id, "canonical")),
    );
    observe(
        &mut model,
        context,
        2,
        (51..=100).map(|id| details(context, id, "canonical")),
    );
    model.outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                room.clone(),
                Intent::Send {
                    body: text("one"),
                    options: Default::default(),
                },
                OutboxPhase::Queued,
                1,
            ),
            stored(
                2,
                room,
                Intent::Send {
                    body: text("two"),
                    options: Default::default(),
                },
                OutboxPhase::Queued,
                2,
            ),
        ]),
    );
    model.project_history(context);
    let first = model.snapshot.messages.clone();
    assert_eq!(first.as_slice().len(), 100);
    assert_eq!(
        first
            .as_slice()
            .iter()
            .filter(|m| m.delivery == DeliveryState::Pending)
            .count(),
        2
    );
    assert_eq!(
        model
            .history
            .get(context)
            .unwrap()
            .messages
            .as_slice()
            .len(),
        100
    );
    model.project_history(context);
    assert_eq!(model.snapshot.messages, first);
}

#[test]
fn an_unknown_enqueue_completion_keeps_the_same_operation_until_storage_is_reconciled() {
    let (mut model, context, room) = model();
    let operation = OperationId::from_local(1);
    model.outbox.load(
        context.account.account,
        &page([stored(
            1,
            room,
            Intent::Send {
                body: text("retained draft"),
                options: Default::default(),
            },
            OutboxPhase::Queued,
            1,
        )]),
    );
    model.outbox.entries[0].state = State::Saving;
    model.outbox.entries[0].request = Some((RequestId(9), context));
    // Worker/receipt loss does not prove that SQLite failed before commit.
    model.complete_outbox_data(
        Work::Save(context.account.account, operation),
        Err(IdentityError::WorkerUnavailable),
    );
    assert!(
        model.outbox.contains(operation),
        "uncertain commit must retain its stable operation"
    );
    assert!(
        !model
            .snapshot
            .submission
            .is_some_and(|s| matches!(s.result, Completion::Failed(_))),
        "unknown durability cannot be reported as confirmed non-admission"
    );
}

#[test]
fn recovered_send_and_forward_use_read_only_reconciliation_and_keep_unknown_receipts_uncertain() {
    let context = crate::composition::fixtures::context(1);
    for intent in [
        Intent::Send {
            body: text("one"),
            options: Default::default(),
        },
        Intent::Forward {
            message: 7,
            destination: key(2),
            author: text("Synthetic actor"),
            body: text("original"),
        },
    ] {
        assert!(matches!(
            reconcile_command(context, &intent).unwrap().0,
            wire::MessagingCommand::ReconcileSend
        ));
        assert!(!reconciled(
            &intent,
            &wire::MessagingResult::Send(protocol::SendReceipt {
                operation: OperationId::from_local(1),
                state: SendState::Uncertain
            })
        ));
    }
    // Scheduling currently has no server correlation key. A lookalike must
    // not authorize retry or pretend the possibly accepted POST is settled.
    let schedule = Intent::Schedule {
        body: text("identical-looking schedule"),
        timestamp: 500,
        options: Default::default(),
    };
    assert!(matches!(
        reconcile_command(context, &schedule).unwrap().0,
        wire::MessagingCommand::Scheduled
    ));
    assert!(!reconciled(
        &schedule,
        &wire::MessagingResult::Scheduled(Page::empty())
    ));
}

#[test]
fn recovery_rejects_a_foreign_message_before_settling_the_durable_operation() {
    let (mut model, context, room) = model();
    let operation = OperationId::from_local(1);
    model.outbox.load(
        context.account.account,
        &page([stored(
            1,
            room,
            Intent::Send {
                body: text("own intended body"),
                options: Default::default(),
            },
            OutboxPhase::Dispatched,
            1,
        )]),
    );
    model.outbox.entries[0].state = State::Running;
    model
        .history
        .admitted(context, operation, messaging::Work::Send, RequestId(0), 1);
    let mut foreign = details(context, 99, "same-looking foreign body");
    foreign.own = false;
    model.apply_message(
        RequestId(0),
        context,
        operation,
        messaging::Work::Send,
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: wire::MessagingResult::Message(Box::new(foreign)),
        })),
    );
    assert!(model.outbox.entries[0].state == State::Uncertain);
    assert!(
        model
            .history
            .get(context)
            .unwrap()
            .messages
            .as_slice()
            .iter()
            .all(|m| m.local_id != 99)
    );
}

#[test]
fn a_correlated_send_receipt_replaces_the_pending_bubble_with_one_canonical_row() {
    let (mut model, context, room) = model();
    let operation = OperationId::from_local(1);
    observe(&mut model, context, 1, [details(context, 7, "existing")]);
    model.outbox.load(
        context.account.account,
        &page([stored(
            1,
            room,
            Intent::Send {
                body: text("own intended body"),
                options: Default::default(),
            },
            OutboxPhase::Queued,
            1,
        )]),
    );
    model.project_history(context);
    assert_eq!(
        model
            .snapshot
            .messages
            .as_slice()
            .iter()
            .filter(|m| m.delivery == DeliveryState::Pending)
            .count(),
        1
    );
    model.outbox.entries[0].state = State::Running;
    model
        .history
        .admitted(context, operation, messaging::Work::Send, RequestId(0), 2);
    let mut accepted = details(context, 99, "own intended body");
    accepted.message.own_operation = Some(operation);
    model.apply_message(
        RequestId(0),
        context,
        operation,
        messaging::Work::Send,
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: wire::MessagingResult::Message(Box::new(accepted)),
        })),
    );
    model.project_history(context);
    assert_eq!(
        model
            .snapshot
            .messages
            .as_slice()
            .iter()
            .filter(|m| m.local_id == 99)
            .count(),
        1
    );
    assert!(
        model
            .snapshot
            .messages
            .as_slice()
            .iter()
            .all(|m| m.delivery != DeliveryState::Pending)
    );
    assert!(model.outbox.entries[0].state == State::Removing);
}

#[test]
fn session_only_work_retires_with_its_original_runtime_even_when_the_same_account_reconnects() {
    let (mut model, context, room) = model();
    let operation = OperationId::from_local(1);
    model.outbox.load(
        context.account.account,
        &page([stored(
            1,
            room,
            Intent::Delete { message: 7 },
            OutboxPhase::Queued,
            1,
        )]),
    );
    model.outbox.entries[0].durable = false;
    model.outbox.entries[0].context = Some(context);
    model.outbox.entries[0].state = State::Running;
    let replacement = AccountScope {
        runtime: talk_core::ui::RuntimeGeneration::from_local(
            context.account.runtime.as_local() + 1,
        ),
        ..context.account
    };
    model.snapshot.accounts = page(model.snapshot.accounts.as_slice().iter().cloned().map(
        |mut account| {
            if account.scope == context.account {
                account.scope = replacement;
            }
            account
        },
    ));
    model.snapshot.active_account = Some(replacement);
    model.snapshot.selected = None;
    model.snapshot.messages = Page::empty();
    model.tick_outbox();
    assert!(
        !model.outbox.contains(operation),
        "session-memory actions must not acquire a replacement runtime"
    );
}

#[test]
fn a_lost_live_scope_retires_running_authority_without_replaying_the_durable_write() {
    let context = crate::composition::fixtures::context(1);
    let operation = OperationId::from_local(1);
    let mut outbox = Outbox::default();
    outbox.load(
        context.account.account,
        &page([stored(
            1,
            key(1),
            Intent::Send {
                body: text("possible effect"),
                options: Default::default(),
            },
            OutboxPhase::Queued,
            1,
        )]),
    );
    outbox.entries[0].state = State::Running;
    outbox.entries[0].context = Some(context);
    outbox.lost_scope(operation);
    assert!(outbox.entries[0].state == State::Uncertain);
    assert_eq!(outbox.entries[0].record.phase, OutboxPhase::Uncertain);
    assert!(matches!(
        reconcile_command(context, &outbox.entries[0].intent)
            .unwrap()
            .0,
        wire::MessagingCommand::ReconcileSend
    ));
}

#[test]
fn resource_order_survives_retry_and_uncertainty_while_unrelated_messages_can_progress() {
    let context = crate::composition::fixtures::context(1);
    let now = Instant::now();
    for blocked in [
        State::Saving,
        State::Queued,
        State::Ready,
        State::Uncertain,
        State::ParkedUncertain,
        State::Confirming,
    ] {
        let mut outbox = Outbox::default();
        outbox.load(
            context.account.account,
            &page([
                stored(
                    1,
                    key(1),
                    Intent::Edit {
                        message: 7,
                        body: text("older"),
                    },
                    OutboxPhase::Queued,
                    1,
                ),
                stored(
                    2,
                    key(1),
                    Intent::Delete { message: 7 },
                    OutboxPhase::Queued,
                    2,
                ),
                stored(
                    3,
                    key(1),
                    Intent::Reaction {
                        message: 8,
                        emoji: text("👍"),
                        add: true,
                    },
                    OutboxPhase::Queued,
                    3,
                ),
            ]),
        );
        for entry in &mut outbox.entries {
            entry.retry = now;
        }
        outbox.entries[0].state = blocked;
        outbox.entries[0].retry = now + Duration::from_secs(5);
        assert_eq!(outbox.next_index(context.account.account, now), Some(2));
        assert_eq!(
            outbox.next_index(context.account.account, now + Duration::from_secs(5)),
            Some(0)
        );
        outbox.entries[0].state = State::Failed;
        assert_eq!(outbox.next_index(context.account.account, now), Some(1));
    }
}

#[test]
fn outgoing_order_is_per_destination_and_only_one_dispatch_runs_per_account() {
    let context = crate::composition::fixtures::context(1);
    let now = Instant::now();
    let mut outbox = Outbox::default();
    outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                key(1),
                Intent::Send {
                    body: text("first"),
                    options: Default::default(),
                },
                OutboxPhase::Queued,
                1,
            ),
            stored(
                2,
                key(1),
                Intent::Send {
                    body: text("second"),
                    options: Default::default(),
                },
                OutboxPhase::Queued,
                2,
            ),
            stored(
                3,
                key(2),
                Intent::Send {
                    body: text("independent"),
                    options: Default::default(),
                },
                OutboxPhase::Queued,
                3,
            ),
        ]),
    );
    for entry in &mut outbox.entries {
        entry.retry = now;
    }
    outbox.entries[0].state = State::Uncertain;
    outbox.entries[0].retry = now + Duration::from_secs(30);
    assert_eq!(outbox.next_index(context.account.account, now), Some(2));
    outbox.entries[2].state = State::Running;
    assert_eq!(outbox.next_index(context.account.account, now), None);
    outbox.entries[2].state = State::Removing;
    assert_eq!(outbox.next_index(context.account.account, now), Some(2));
}

#[test]
fn a_failed_reconciliation_read_never_turns_the_original_write_ready_for_replay() {
    let context = crate::composition::fixtures::context(1);
    for reason in [
        UnavailableReason::Busy,
        UnavailableReason::Connection(ConnectionFailure::Unreachable),
        UnavailableReason::Connection(ConnectionFailure::TimedOut),
        UnavailableReason::Connection(ConnectionFailure::ServerUnavailable),
    ] {
        let mut model = crate::composition::accounts::tests::model();
        let operation = OperationId::from_local(1);
        model.outbox.load(
            context.account.account,
            &page([stored(
                1,
                key(1),
                Intent::Send {
                    body: text("possibly applied"),
                    options: Default::default(),
                },
                OutboxPhase::Dispatched,
                1,
            )]),
        );
        model.outbox.entries[0].state = State::Running;
        model.outbox_result(operation, &Err(Completion::Failed(reason)));
        let entry = &model.outbox.entries[0];
        assert!(
            entry.state == State::Uncertain,
            "a failed GET cannot authorize replaying POST for {reason:?}"
        );
        assert_eq!(entry.record.phase, OutboxPhase::Uncertain);
        assert!(matches!(
            reconcile_command(context, &entry.intent).unwrap().0,
            wire::MessagingCommand::ReconcileSend
        ));
    }
}

#[test]
fn optimistic_deletion_redacts_parent_aliases_in_every_projected_message_collection() {
    let context = crate::composition::fixtures::context(1);
    let mut quoted = details(context, 8, "reply");
    quoted.parent = Some(wire::ReplyContext {
        id: MessageId {
            conversation: context.conversation,
            sequence: 7,
        },
        author: text("Synthetic actor"),
        text: text("deleted body"),
        unavailable: false,
    });
    let quote = messaging::message_view(&quoted);
    let mut messages = page([
        messaging::message_view(&details(context, 7, "deleted body")),
        quote.clone(),
    ]);
    let mut panel = view::MessagePanel::new(context);
    panel.thread_messages = page([quote.clone()]);
    panel.pinned_messages = page([quote]);
    let mut outbox = Outbox::default();
    let room = key(1);
    outbox.load(
        context.account.account,
        &page([stored(
            1,
            room.clone(),
            Intent::Delete { message: 7 },
            OutboxPhase::Queued,
            1,
        )]),
    );
    outbox.project(context, &room, "Synthetic actor", &mut messages, &mut panel);
    for row in messages
        .as_slice()
        .iter()
        .chain(panel.thread_messages.as_slice())
        .chain(panel.pinned_messages.as_slice())
        .filter(|row| row.local_id == 8)
    {
        let parent = row.metadata.parent.as_ref().unwrap();
        assert!(parent.unavailable);
        assert_eq!(parent.text.as_str(), "");
        assert_eq!(parent.author.as_str(), "");
    }
}

#[test]
fn a_mismatching_or_forbidden_read_preserves_outcome_uncertainty_without_overwriting_new_canonical_data()
 {
    for read_failure in [false, true] {
        let (mut model, context, room) = model();
        let operation = OperationId::from_local(1);
        observe(
            &mut model,
            context,
            1,
            [details(context, 7, "newer canonical")],
        );
        model.outbox.load(
            context.account.account,
            &page([stored(
                1,
                room,
                Intent::Edit {
                    message: 7,
                    body: text("possibly accepted earlier"),
                },
                OutboxPhase::Dispatched,
                1,
            )]),
        );
        model.outbox.entries[0].state = State::Running;
        let result = if read_failure {
            Err(Completion::Failed(UnavailableReason::PermissionDenied))
        } else {
            Ok(wire::MessagingResult::Message(Box::new(details(
                context,
                7,
                "newer canonical",
            ))))
        };
        model.outbox_result(operation, &result);
        let entry = &model.outbox.entries[0];
        assert!(
            !matches!(entry.state, State::Failed | State::Removing | State::Ready),
            "an inconclusive GET cannot classify or repeat the older write"
        );
        assert_eq!(entry.record.phase, OutboxPhase::Uncertain);
        model.project_history(context);
        assert_eq!(
            model.snapshot.messages.as_slice()[0].body.as_str(),
            "newer canonical"
        );
        assert_eq!(
            model
                .snapshot
                .message_panel
                .as_ref()
                .unwrap()
                .action_error
                .unwrap()
                .1,
            view::MessageActionError::Uncertain
        );
    }
}

#[test]
fn each_failed_action_has_a_new_notice_but_an_uncertain_operation_does_not_repeat_its_toast() {
    let (mut model, context, room) = model();
    model.outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                room.clone(),
                Intent::Delete { message: 7 },
                OutboxPhase::Queued,
                1,
            ),
            stored(
                2,
                room.clone(),
                Intent::Delete { message: 8 },
                OutboxPhase::Queued,
                2,
            ),
            stored(
                3,
                room,
                Intent::Send {
                    body: text("possibly applied"),
                    options: Default::default(),
                },
                OutboxPhase::Dispatched,
                3,
            ),
        ]),
    );
    model.outbox_result(
        OperationId::from_local(1),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied)),
    );
    let first = model.outbox.errors.last().unwrap().2;
    model.outbox_result(
        OperationId::from_local(2),
        &Err(Completion::Failed(UnavailableReason::PermissionDenied)),
    );
    let second = model.outbox.errors.last().unwrap().2;
    assert!(second > first);
    model.outbox_result(OperationId::from_local(3), &Err(Completion::Uncertain));
    let uncertain = model.outbox.errors.last().unwrap().2;
    assert!(uncertain > second);
    assert_eq!(
        model.outbox.errors.last().unwrap().3,
        view::MessageActionError::Uncertain
    );
    model.outbox_result(OperationId::from_local(3), &Err(Completion::Uncertain));
    assert_eq!(model.outbox.errors.last().unwrap().2, uncertain);
}

#[test]
fn persisted_failed_send_edit_and_schedule_restore_as_recoverable_drafts_without_auto_dispatch() {
    let (mut model, context, room) = model();
    observe(
        &mut model,
        context,
        1,
        [details(context, 7, "server canonical")],
    );
    model.outbox.load(
        context.account.account,
        &page([
            stored(
                1,
                room.clone(),
                Intent::Send {
                    body: text("failed send retained"),
                    options: Default::default(),
                },
                OutboxPhase::Failed,
                1,
            ),
            stored(
                2,
                room.clone(),
                Intent::Edit {
                    message: 7,
                    body: text("failed edit retained"),
                },
                OutboxPhase::Failed,
                2,
            ),
            stored(
                3,
                room,
                Intent::Schedule {
                    body: text("failed schedule retained"),
                    timestamp: 500,
                    options: Default::default(),
                },
                OutboxPhase::Failed,
                3,
            ),
        ]),
    );
    assert!(
        model
            .outbox
            .entries
            .iter()
            .all(|entry| entry.state == State::Failed && entry.failed_saved)
    );
    assert_eq!(
        model.outbox.next_index(
            context.account.account,
            Instant::now() + Duration::from_secs(60)
        ),
        None
    );
    model.project_history(context);
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.failed_drafts.as_slice().len(), 3);
    assert_eq!(
        panel.failed_drafts.as_slice()[0].body.as_str(),
        "failed send retained"
    );
    assert!(matches!(
        panel.failed_drafts.as_slice()[1].kind,
        view::FailedDraftKind::Edit(7)
    ));
    assert!(matches!(
        panel.failed_drafts.as_slice()[2].kind,
        view::FailedDraftKind::Schedule(500)
    ));
    assert!(
        panel
            .failed_drafts
            .as_slice()
            .iter()
            .all(|draft| !draft.retrying)
    );
    assert_eq!(model.snapshot.messages.as_slice().len(), 1);
    assert_eq!(
        model.snapshot.messages.as_slice()[0].body.as_str(),
        "server canonical"
    );
    // Rendering and revisiting the room do not acknowledge the durable text.
    model.project_history(context);
    assert_eq!(model.outbox.entries.len(), 3);
}
