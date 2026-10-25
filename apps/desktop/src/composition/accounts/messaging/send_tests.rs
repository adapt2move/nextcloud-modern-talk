use super::*;

fn queued(now: Instant) -> QueuedSend {
    QueuedSend {
        request: RequestId(42),
        context: crate::composition::fixtures::context(1),
        body: Text::new("Retained synthetic send").unwrap(),
        target: view::ComposeTarget {
            reply_to: Some(7),
            thread: Some(3),
            silent: true,
        },
        deadline: now + Duration::from_secs(10),
    }
}
fn model(now: Instant) -> Accounts {
    let mut model = crate::composition::accounts::tests::model();
    model.snapshot = crate::composition::fixtures::snapshot(Some("chat"));
    model.snapshot.selected = Some(queued(now).context);
    model.snapshot.message_panel = Some(view::MessagePanel::new(queued(now).context));
    model.queued_send = Some(queued(now));
    model
}

#[test]
fn busy_attempts_keep_the_exact_body_target_and_request_until_one_admission() {
    let now = Instant::now();
    let mut slot = Some(queued(now));
    let mut attempts = 0;
    for millis in [0, 100, 200] {
        let job = slot.take().unwrap();
        let admitted = job
            .attempt(
                Some(job.context),
                now + Duration::from_millis(millis),
                |job| {
                    attempts += 1;
                    assert_eq!(job.request, RequestId(42));
                    assert_eq!(job.body.as_str(), "Retained synthetic send");
                    assert_eq!(job.target.reply_to, Some(7));
                    assert_eq!(job.target.thread, Some(3));
                    assert!(job.target.silent);
                    if attempts < 3 {
                        Err(SubmitError::Busy)
                    } else {
                        Ok(())
                    }
                },
            )
            .unwrap();
        if !admitted {
            slot = Some(job);
        }
    }
    assert!(slot.is_none(), "admitted writes leave the retry slot");
    assert_eq!(attempts, 3);
}

#[test]
fn expiry_navigation_and_replaced_runtime_never_dispatch_a_waiting_send() {
    let now = Instant::now();
    let job = queued(now);
    assert_eq!(
        job.attempt(Some(job.context), job.deadline, |_| panic!(
            "expired dispatch"
        )),
        Err(SubmitError::Unavailable(UnavailableReason::Busy))
    );
    let mut other_runtime = job.context;
    other_runtime.account.runtime = talk_core::ids::RuntimeGeneration::from_local(2);
    for selected in [
        None,
        Some(crate::composition::fixtures::context(2)),
        Some(other_runtime),
    ] {
        assert_eq!(
            job.attempt(selected, now, |_| panic!("obsolete dispatch")),
            Err(SubmitError::StaleScope)
        );
    }
}

#[test]
fn a_non_busy_rejection_never_becomes_an_automatic_retry() {
    let now = Instant::now();
    let job = queued(now);
    for error in [
        SubmitError::StaleScope,
        SubmitError::Unavailable(UnavailableReason::PermissionDenied),
        SubmitError::Unavailable(UnavailableReason::ReauthenticationRequired),
    ] {
        assert_eq!(
            job.attempt(Some(job.context), now, |_| Err(error)),
            Err(error)
        );
    }
}

#[test]
fn owner_waits_for_other_actions_then_reports_a_correlated_terminal_failure() {
    let now = Instant::now();
    let mut model = model(now);
    let context = model.snapshot.selected.unwrap();
    model.snapshot.message_panel.as_mut().unwrap().state = RemoteState::Saving(RequestId(1));
    model.tick_queued_send(now);
    assert!(model.queued_send.is_some());
    assert!(model.snapshot.submission.is_none());
    assert!(model.unresolved.is_empty());
    model.tick_queued_send(now + Duration::from_secs(10));
    assert!(model.queued_send.is_none());
    assert_eq!(
        model.snapshot.submission,
        Some(SubmissionResult {
            request: RequestId(42),
            context,
            result: Completion::Failed(UnavailableReason::Busy),
        })
    );
}

#[test]
fn owner_preserves_unacknowledged_results_and_cancels_after_navigation() {
    let now = Instant::now();
    let mut model = model(now);
    let context = model.snapshot.selected.unwrap();
    let previous = SubmissionResult {
        request: RequestId(1),
        context,
        result: Completion::Uncertain,
    };
    model.snapshot.submission = Some(previous);
    model.tick_queued_send(now);
    assert!(model.queued_send.is_some());
    assert_eq!(model.snapshot.submission, Some(previous));
    model.snapshot.submission = None;
    model.snapshot.selected = Some(crate::composition::fixtures::context(2));
    model.tick_queued_send(now);
    assert!(model.queued_send.is_none());
    assert_eq!(
        model.snapshot.submission,
        Some(SubmissionResult {
            request: RequestId(42),
            context,
            result: Completion::Failed(UnavailableReason::PolicyStale),
        })
    );
    assert!(model.pending.is_empty());
}

#[test]
fn observed_offline_state_cancels_before_dispatch_instead_of_sending_on_reconnect() {
    let now = Instant::now();
    let mut model = model(now);
    model.snapshot.content = ContentState::Offline;
    model.tick_queued_send(now);
    assert!(model.queued_send.is_none());
    assert!(matches!(
        model.snapshot.submission,
        Some(SubmissionResult {
            result: Completion::Failed(UnavailableReason::PolicyStale),
            ..
        })
    ));
    assert!(model.pending.is_empty());
}

#[test]
fn another_in_flight_send_cannot_overwrite_the_shared_completion_slot() {
    let now = Instant::now();
    let mut model = model(now);
    let context = model.snapshot.selected.unwrap();
    model.queued_send = None;
    model.pending.push(Pending {
        id: ConnectionRequestId(1),
        purpose: Purpose::Message(
            RequestId(1),
            crate::composition::fixtures::context(2),
            OperationId::from_local(1),
            Work::Send,
        ),
    });
    assert_eq!(
        model.dispatch_message_action(
            RequestId(42),
            context,
            view::MessageAction::Send {
                body: Text::new("Next explicit send").unwrap(),
                target: view::ComposeTarget::default(),
            }
        ),
        Err(SubmitError::Busy)
    );
    assert!(model.unresolved.is_empty());
    assert_eq!(
        model.submit(UiIntent::SendDraft {
            request: RequestId(43),
            context,
            body: Text::new("Legacy route must use the same capacity").unwrap(),
        }),
        Err(SubmitError::Busy)
    );
}
