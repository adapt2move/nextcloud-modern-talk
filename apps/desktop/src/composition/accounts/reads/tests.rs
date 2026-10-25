use super::*;

fn job(now: Instant) -> QueuedRead {
    QueuedRead {
        request: RequestId(42),
        context: crate::composition::fixtures::context(1),
        read: Read::Thread(7),
        deadline: now + Duration::from_secs(10),
    }
}
fn model() -> Accounts {
    let mut model = super::super::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
    let context = crate::composition::fixtures::context(1);
    model.snapshot.selected = Some(context);
    model.snapshot.message_panel = Some(view::MessagePanel::new(context));
    model.history.open(context);
    model
}
fn occupied(model: &mut Accounts, work: Work) {
    let context = model.snapshot.selected.unwrap();
    model.pending.push(Pending {
        id: ConnectionRequestId(1),
        purpose: Purpose::Message(RequestId(1), context, OperationId::from_local(1), work),
    });
}

#[test]
fn only_busy_nonadmission_retries_the_exact_request_and_target_without_extending_its_deadline() {
    let now = Instant::now();
    let mut pending = Some(job(now));
    let mut attempts = 0;
    for offset in [0, 50, 100] {
        let read = pending.take().unwrap();
        let admitted = read
            .attempt(
                Some(read.context),
                now + Duration::from_millis(offset),
                |read| {
                    attempts += 1;
                    assert_eq!(read.request, RequestId(42));
                    assert_eq!(read.read, Read::Thread(7));
                    assert_eq!(read.deadline, now + Duration::from_secs(10));
                    if attempts < 3 {
                        Err(SubmitError::Busy)
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap();
        if !admitted {
            pending = Some(read);
        }
    }
    assert!(pending.is_none());
    assert_eq!(attempts, 3);
    let read = job(now);
    for error in [
        SubmitError::StaleScope,
        SubmitError::Unavailable(UnavailableReason::PermissionDenied),
        SubmitError::Unavailable(UnavailableReason::Unsupported),
    ] {
        assert_eq!(
            read.attempt(Some(read.context), now, |_| Err(error)),
            Err(error)
        );
    }
}
#[test]
fn expiry_and_navigation_or_runtime_replacement_never_dispatch_a_waiting_get() {
    let now = Instant::now();
    let read = job(now);
    assert_eq!(
        read.attempt(Some(read.context), read.deadline, |_| panic!("expired GET")),
        Err(SubmitError::Unavailable(UnavailableReason::Busy))
    );
    let mut replaced = read.context;
    replaced.account.runtime = talk_core::ids::RuntimeGeneration::from_local(99);
    for selected in [
        None,
        Some(crate::composition::fixtures::context(2)),
        Some(replaced),
    ] {
        assert_eq!(
            read.attempt(selected, now, |_| panic!("obsolete GET")),
            Err(SubmitError::StaleScope)
        );
    }
}
#[test]
fn queued_thread_is_locally_accepted_and_correlated_before_any_operation_is_admitted() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    occupied(&mut model, Work::Thread);
    assert_eq!(
        model.message_action(RequestId(42), context, view::MessageAction::Thread(7)),
        Ok(())
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.thread_root, Some(7));
    assert_eq!(panel.thread_state, RemoteState::Loading(RequestId(42)));
    assert_eq!(model.pending.len(), 1);
    let deadline = model.queued_read.queued.as_ref().unwrap().deadline;
    model.tick_queued_read(deadline);
    assert!(model.queued_read.queued.is_none());
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().thread_state,
        RemoteState::Failed(RequestId(42), UnavailableReason::Busy)
    );
    assert_eq!(model.pending.len(), 1);
}
#[test]
fn newest_target_supersedes_the_single_slot_and_never_steals_a_mutation_lease() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    occupied(&mut model, Work::Thread);
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(99));
    model
        .message_action(RequestId(42), context, view::MessageAction::Thread(7))
        .unwrap();
    model
        .message_action(RequestId(43), context, view::MessageAction::Thread(8))
        .unwrap();
    assert_eq!(
        model.queued_read.queued.as_ref().unwrap().read,
        Read::Thread(8)
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.thread_root, Some(8));
    assert_eq!(panel.thread_state, RemoteState::Loading(RequestId(43)));
    assert_eq!(panel.state, RemoteState::Saving(RequestId(99)));
    model.snapshot.selected = Some(crate::composition::fixtures::context(2));
    model.tick_queued_read(Instant::now());
    assert!(model.queued_read.queued.is_none());
    assert_eq!(model.pending.len(), 1);
}
#[test]
fn pin_loading_and_terminal_busy_feedback_survive_canonical_history_projection() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    occupied(&mut model, Work::Pins);
    model
        .message_action(RequestId(42), context, view::MessageAction::Pins)
        .unwrap();
    model.project_history(context);
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().pins_state,
        RemoteState::Loading(RequestId(42))
    );
    let deadline = model.queued_read.queued.as_ref().unwrap().deadline;
    model.tick_queued_read(deadline);
    model.project_history(context);
    assert_eq!(
        model.snapshot.message_panel.as_ref().unwrap().pins_state,
        RemoteState::Failed(RequestId(42), UnavailableReason::Busy)
    );
    model.clear_content();
    assert!(model.queued_read.queued.is_none() && model.queued_read.failed.is_none());
}
#[test]
fn unavailable_initial_read_has_a_terminal_target_state_and_never_becomes_a_retry() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    model
        .message_action(RequestId(42), context, view::MessageAction::Thread(7))
        .unwrap();
    assert!(model.queued_read.queued.is_none());
    assert!(matches!(
        model.snapshot.message_panel.as_ref().unwrap().thread_state,
        RemoteState::Failed(RequestId(42), _)
    ));
    assert!(model.pending.is_empty());
}
#[test]
fn pagination_metadata_is_captured_and_no_write_can_enter_the_read_slot() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    model.message_search = Some((
        context,
        text("retained query"),
        Some(text("retained cursor")),
    ));
    let read = Read::capture(&model, context, &view::MessageAction::SearchNext)
        .unwrap()
        .unwrap();
    model.message_search = None;
    assert_eq!(
        read,
        Read::Search {
            query: text("retained query"),
            cursor: Some(text("retained cursor"))
        }
    );
    assert!(
        matches!(read.command(context),Ok(wire::MessagingCommand::Search{query,cursor:Some(cursor)}) if query.as_str()=="retained query"&&cursor.as_str()=="retained cursor")
    );
    for action in [
        view::MessageAction::Send {
            body: text("body"),
            target: Default::default(),
        },
        view::MessageAction::Edit {
            message: 7,
            body: text("edit"),
        },
        view::MessageAction::Delete(7),
        view::MessageAction::Pin(7),
        view::MessageAction::React {
            message: 7,
            emoji: text("👍"),
            add: true,
        },
        view::MessageAction::Reminder {
            message: 7,
            timestamp: Some(10),
        },
        view::MessageAction::Schedule {
            body: text("later"),
            timestamp: 10,
            target: Default::default(),
        },
        view::MessageAction::MarkRead(7),
        view::MessageAction::Typing(true),
    ] {
        assert!(Read::capture(&model, context, &action).is_none());
    }
}

#[test]
fn superseded_shared_read_cannot_replace_results_or_cursor_during_an_unrelated_write() {
    let mut model = model();
    let context = model.snapshot.selected.unwrap();
    let operation = OperationId::from_local(7);
    model
        .history
        .admitted(context, operation, Work::Search, RequestId(1), 1);
    model.queued_read.shared = Some((context, RequestId(2), Work::Search));
    model.message_search = Some((context, text("new query"), Some(text("new cursor"))));
    let panel = model.snapshot.message_panel.as_mut().unwrap();
    panel.state = RemoteState::Saving(RequestId(99));
    panel.search = page([view::SearchResult {
        id: 7,
        title: text("Retained result"),
        excerpt: text("Retained excerpt"),
    }]);
    let retained = panel.search.clone();
    model.apply_message(
        RequestId(1),
        context,
        operation,
        Work::Search,
        Ok(RuntimeReply::Protocol(ProtocolReply::Messaging {
            conversation: context.conversation,
            operation,
            result: wire::MessagingResult::Search {
                hits: Page::empty(),
                cursor: Some(text("obsolete cursor")),
            },
        })),
    );
    let panel = model.snapshot.message_panel.as_ref().unwrap();
    assert_eq!(panel.search, retained);
    assert_eq!(panel.state, RemoteState::Saving(RequestId(99)));
    assert_eq!(
        model
            .message_search
            .as_ref()
            .unwrap()
            .2
            .as_ref()
            .unwrap()
            .as_str(),
        "new cursor"
    );
}
