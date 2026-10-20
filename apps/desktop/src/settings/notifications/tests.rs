use super::*;
use talk_core::{
    bounded::Text,
    ids::{AccountId, ActorId, CallGeneration, ConversationId, HostId, RuntimeGeneration},
    preferences::{
        NotificationDelivery,
        notifications::{NotificationActivation, NotificationId},
    },
};

fn scope(account: u128, runtime: u128) -> AccountScope {
    AccountScope {
        account: AccountId::from_local(account),
        runtime: RuntimeGeneration::from_local(runtime),
    }
}
fn call(scope: AccountScope, generation: u128) -> CallScope {
    CallScope {
        account: scope,
        conversation: ConversationId {
            account: scope.account,
            host: HostId::from_local(1),
            local: 1,
        },
        call: CallGeneration::from_local(generation),
    }
}
fn sources(scope: AccountScope, revision: u64) -> NotificationSources {
    NotificationSources {
        scope,
        revision,
        messages: SourceAvailability::Available,
        important: SourceAvailability::Available,
        incoming_calls: SourceAvailability::Available,
        calendar: SourceAvailability::Unavailable,
        ringing: Some(call(scope, 1)),
    }
}
fn event(request: u64, kind: NotificationKind) -> NotificationEvent {
    NotificationEvent {
        request: RequestId(request),
        kind,
        title: Text::new("Notification policy test").unwrap(),
        body: Text::new("Synthetic local fixture; no live account").unwrap(),
        minimized: true,
        eligible: true,
    }
}
fn test(request: u64, category: NotificationCategory) -> NotificationEvent {
    NotificationEvent::test(
        RequestId(request),
        category,
        Text::new("Notification test").unwrap(),
        Text::new("No account event").unwrap(),
    )
}
fn message(scope: AccountScope, id: u64) -> NotificationKind {
    NotificationKind::Message {
        scope,
        conversation: call(scope, 1).conversation,
        id: NotificationId(id),
    }
}
fn join(scope: AccountScope, id: u64, is_self: bool) -> NotificationKind {
    NotificationKind::ConferenceParticipantJoined {
        call: call(scope, 1),
        participant: ActorId::from_local(id as u128),
        is_self,
        id: NotificationId(id),
    }
}
fn finish_next(state: &mut State) -> NotificationEvent {
    let next = state.next().unwrap();
    state.running = None;
    state.finish(
        &next,
        DeliveryOutcome::Suppressed,
        DeliveryOutcome::Suppressed,
    );
    next
}
fn delivery_mut(
    policy: &mut NotificationPreferences,
    category: NotificationCategory,
) -> &mut NotificationDelivery {
    match category {
        NotificationCategory::Messages => &mut policy.messages,
        NotificationCategory::Important => &mut policy.important,
        NotificationCategory::IncomingCalls => &mut policy.incoming_calls,
        NotificationCategory::Calendar => &mut policy.calendar,
    }
}

#[test]
fn every_category_obeys_its_own_switches_source_and_eligibility() {
    for category in [
        NotificationCategory::Messages,
        NotificationCategory::Important,
        NotificationCategory::IncomingCalls,
        NotificationCategory::Calendar,
    ] {
        let mut policy = NotificationPreferences::default();
        let mut notification = test(1, category);
        if category == NotificationCategory::Calendar {
            assert_eq!(policy.calendar, NotificationDelivery::OFF);
            assert!(
                !notifications::plan(policy, &notification, SourceAvailability::Available).banner
            );
        }
        for banners in [false, true] {
            for sounds in [false, true] {
                *delivery_mut(&mut policy, category) = NotificationDelivery { banners, sounds };
                assert_eq!(
                    notifications::plan(policy, &notification, SourceAvailability::Available),
                    notifications::NotificationPlan {
                        banner: banners,
                        sound: sounds
                    }
                );
                for unavailable in [SourceAvailability::Unknown, SourceAvailability::Unavailable] {
                    assert_eq!(
                        notifications::plan(policy, &notification, unavailable),
                        notifications::NotificationPlan {
                            banner: false,
                            sound: false
                        }
                    );
                }
            }
        }
        notification.eligible = false;
        assert!(!notifications::plan(policy, &notification, SourceAvailability::Available).sound);
        notification.eligible = true;
        notification.minimized = false;
        assert_eq!(
            notifications::plan(policy, &notification, SourceAvailability::Available).banner,
            category == NotificationCategory::IncomingCalls
        );
    }
    let policy = NotificationPreferences {
        messages: NotificationDelivery::OFF,
        ..NotificationPreferences::default()
    };
    assert!(
        notifications::plan(
            policy,
            &test(2, NotificationCategory::IncomingCalls),
            SourceAvailability::Available
        )
        .sound
    );
}

#[test]
fn conference_and_direct_call_routes_are_distinct_and_self_join_is_silent() {
    let account = scope(1, 1);
    let conference = event(1, join(account, 1, false));
    assert_eq!(
        conference.kind.activation(),
        Some(NotificationActivation::OpenConferencePrejoin(call(
            account, 1
        )))
    );
    assert_eq!(
        NotificationKind::IncomingCall {
            call: call(account, 1)
        }
        .activation(),
        Some(NotificationActivation::AcceptDirectCall(call(account, 1)))
    );
    let own = event(2, join(account, 2, true));
    assert_eq!(
        notifications::plan(
            NotificationPreferences::default(),
            &own,
            SourceAvailability::Available
        ),
        notifications::NotificationPlan {
            banner: false,
            sound: false
        }
    );
    assert_eq!(
        test(3, NotificationCategory::IncomingCalls)
            .kind
            .activation(),
        None
    );
    assert!(!native_capabilities().native_call_ui);
    assert!(!native_capabilities().native_call_actions);
}

#[test]
fn source_revisions_fence_account_switches_and_calendar_tests_need_evidence() {
    let mut state = State::new(NotificationPreferences::default());
    let first = scope(1, 1);
    let other = scope(2, 1);
    let now = Instant::now();
    assert_eq!(
        state.source(None, NotificationCategory::Calendar),
        SourceAvailability::Unavailable
    );
    assert_eq!(
        state.admit(test(1, NotificationCategory::Calendar), now),
        Err(NotificationError::SourceUnavailable)
    );
    state.update_sources(None, sources(first, 1)).unwrap();
    state.update_sources(None, sources(other, 1)).unwrap();
    assert_eq!(
        state.admit(
            event(
                1,
                NotificationKind::Calendar {
                    scope: first,
                    id: NotificationId(1)
                }
            ),
            now
        ),
        Err(NotificationError::SourceUnavailable)
    );
    let mut calendar_source = sources(first, 2);
    calendar_source.calendar = SourceAvailability::Available;
    state.update_sources(Some(first), calendar_source).unwrap();
    assert_eq!(
        state.source(Some(other), NotificationCategory::Calendar),
        SourceAvailability::Unavailable
    );
    let mut calendar_test = test(1, NotificationCategory::Calendar);
    calendar_test.kind = NotificationKind::Test {
        category: NotificationCategory::Calendar,
        scope: Some(first),
    };
    state.admit(calendar_test, now).unwrap();
    let replacement = scope(1, 2);
    state
        .update_sources(Some(first), sources(replacement, 1))
        .unwrap();
    assert!(state.pending.iter().all(Option::is_none));
    assert_eq!(
        state.completions.pop_front().unwrap().banner,
        DeliveryOutcome::Cancelled
    );
    assert_eq!(
        state.update_sources(Some(first), calendar_source),
        Err(NotificationError::Stale)
    );
    assert_eq!(
        state.update_sources(Some(replacement), sources(replacement, 1)),
        Err(NotificationError::Stale)
    );
    let wrong = event(
        2,
        NotificationKind::Message {
            scope: first,
            conversation: call(other, 1).conversation,
            id: NotificationId(2),
        },
    );
    assert_eq!(state.admit(wrong, now), Err(NotificationError::Stale));
}

#[test]
fn retired_sources_cancel_work_and_free_slots_without_retiring_a_replacement() {
    let mut state = State::new(NotificationPreferences::default());
    for id in 1..=8 {
        state
            .update_sources(None, sources(scope(id, 1), 1))
            .unwrap();
    }
    let now = Instant::now();
    state.admit(event(1, message(scope(1, 1), 1)), now).unwrap();
    state.next().unwrap();
    state.admit(event(2, message(scope(2, 1), 1)), now).unwrap();
    assert_eq!(
        state.update_sources(None, sources(scope(9, 1), 1)),
        Err(NotificationError::Busy)
    );

    state.retire_source(scope(1, 1)).unwrap();
    assert!(state.running.as_ref().unwrap().cancelled);
    state.update_sources(None, sources(scope(9, 1), 1)).unwrap();
    state.retire_source(scope(2, 1)).unwrap();
    assert!(state.pending.iter().all(Option::is_none));
    assert_eq!(
        state.completions.pop_front().unwrap().banner,
        DeliveryOutcome::Cancelled
    );
    assert_eq!(
        state.source(Some(scope(1, 1)), NotificationCategory::Messages),
        SourceAvailability::Unknown
    );

    state
        .update_sources(Some(scope(3, 1)), sources(scope(3, 2), 1))
        .unwrap();
    assert_eq!(
        state.retire_source(scope(3, 1)),
        Err(NotificationError::Stale)
    );
    assert_eq!(
        state.source(Some(scope(3, 2)), NotificationCategory::Messages),
        SourceAvailability::Available
    );
    state
        .update_sources(None, sources(scope(10, 1), 1))
        .unwrap();
    assert_eq!(state.sources.len(), 8);
}

#[test]
fn duplicates_and_participation_bursts_do_not_swallow_personal_reminders() {
    let account = scope(1, 1);
    let mut state = State::new(NotificationPreferences::default());
    state.update_sources(None, sources(account, 1)).unwrap();
    let now = Instant::now();
    state.admit(event(1, message(account, 1)), now).unwrap();
    finish_next(&mut state);
    assert_eq!(
        state.admit(event(2, message(account, 1)), now),
        Err(NotificationError::Duplicate)
    );
    state.admit(event(3, join(account, 1, false)), now).unwrap();
    finish_next(&mut state);
    assert_eq!(
        state.admit(event(4, join(account, 2, false)), now),
        Err(NotificationError::Coalesced)
    );
    let reminder = NotificationKind::MessageReminder {
        scope: account,
        conversation: call(account, 1).conversation,
        id: NotificationId(2),
    };
    state.admit(event(5, reminder), now).unwrap();
    assert_eq!(finish_next(&mut state).kind, reminder);
    state
        .admit(event(6, join(account, 3, false)), now + JOIN_WINDOW)
        .unwrap();
}

#[test]
fn ordinary_overload_reserves_call_capacity_and_preempts_running_helper() {
    let shared = Shared {
        state: Mutex::new(State::new(NotificationPreferences::default())),
        wake: Condvar::new(),
    };
    let now = Instant::now();
    let running;
    {
        let mut state = shared.lock();
        state
            .admit(test(1, NotificationCategory::Messages), now)
            .unwrap();
        running = state.next().unwrap();
        state
            .admit(test(2, NotificationCategory::Messages), now)
            .unwrap();
        assert_eq!(
            state.admit(test(3, NotificationCategory::Messages), now),
            Err(NotificationError::Busy)
        );
        state
            .admit(test(4, NotificationCategory::Important), now)
            .unwrap();
        state
            .admit(test(5, NotificationCategory::IncomingCalls), now)
            .unwrap();
        assert!(state.running.as_ref().unwrap().cancelled);
    }
    assert_eq!(
        check(&shared, &running, Effect::Sound),
        Err(NotificationError::Cancelled)
    );
    let mut state = shared.lock();
    state.running = None;
    state.finish(
        &running,
        DeliveryOutcome::Cancelled,
        DeliveryOutcome::Cancelled,
    );
    assert_eq!(finish_next(&mut state).request, RequestId(5));
    assert_eq!(finish_next(&mut state).request, RequestId(4));
    assert_eq!(finish_next(&mut state).request, RequestId(2));
    assert_eq!(state.completions.len(), 4);
}

#[test]
fn completion_backpressure_is_bounded_with_one_reserved_call_result() {
    let now = Instant::now();
    let mut state = State::new(NotificationPreferences::default());
    for request in 1..COMPLETIONS as u64 {
        state
            .admit(test(request, NotificationCategory::Messages), now)
            .unwrap();
        finish_next(&mut state);
    }
    assert_eq!(
        state.admit(test(16, NotificationCategory::Important), now),
        Err(NotificationError::Busy)
    );
    state
        .admit(test(17, NotificationCategory::IncomingCalls), now)
        .unwrap();
    finish_next(&mut state);
    assert_eq!(state.completions.len(), COMPLETIONS);
    assert_eq!(
        state.admit(test(18, NotificationCategory::IncomingCalls), now),
        Err(NotificationError::Busy)
    );
    state.completions.pop_front();
    state
        .admit(test(19, NotificationCategory::IncomingCalls), now)
        .unwrap();
}

#[test]
fn ended_call_cancels_work_and_old_source_cannot_reopen_it() {
    let account = scope(1, 1);
    let shared = Arc::new(Shared {
        state: Mutex::new(State::new(NotificationPreferences::default())),
        wake: Condvar::new(),
    });
    let runtime = NotificationRuntime {
        shared: Arc::clone(&shared),
    };
    runtime.update_sources(None, sources(account, 1)).unwrap();
    runtime
        .submit(event(
            1,
            NotificationKind::IncomingCall {
                call: call(account, 1),
            },
        ))
        .unwrap();
    runtime.end_call(call(account, 1)).unwrap();
    assert_eq!(
        runtime.take_completion().unwrap().banner,
        DeliveryOutcome::Cancelled
    );
    assert_eq!(
        runtime.update_sources(Some(account), sources(account, 1)),
        Err(NotificationError::Stale)
    );
    assert_eq!(
        runtime.submit(event(
            2,
            NotificationKind::IncomingCall {
                call: call(account, 1)
            }
        )),
        Err(NotificationError::Stale)
    );
    let mut next = sources(account, 2);
    next.ringing = Some(call(account, 2));
    runtime.update_sources(Some(account), next).unwrap();
    runtime
        .submit(event(
            3,
            NotificationKind::IncomingCall {
                call: call(account, 2),
            },
        ))
        .unwrap();
    let running = shared.lock().next().unwrap();
    runtime.end_call(call(account, 2)).unwrap();
    assert_eq!(
        check(&shared, &running, Effect::Banner),
        Err(NotificationError::Cancelled)
    );
}

#[test]
fn linux_commands_keep_every_category_at_normal_focus_controlled_urgency() {
    let account = scope(1, 1);
    let kinds = [
        (message(account, 1), "im.received", "--urgency=normal"),
        (
            join(account, 1, false),
            "x-nextcloud-modern-talk.conference.joined",
            "--urgency=normal",
        ),
        (
            NotificationKind::MessageReminder {
                scope: account,
                conversation: call(account, 1).conversation,
                id: NotificationId(1),
            },
            "x-nextcloud-modern-talk.message.reminder",
            "--urgency=normal",
        ),
        (
            NotificationKind::Test {
                category: NotificationCategory::Important,
                scope: None,
            },
            "x-nextcloud-modern-talk.message.reminder",
            "--urgency=normal",
        ),
        (
            NotificationKind::IncomingCall {
                call: call(account, 1),
            },
            "call.incoming",
            "--urgency=normal",
        ),
        (
            NotificationKind::Calendar {
                scope: account,
                id: NotificationId(1),
            },
            "x-nextcloud-modern-talk.calendar.reminder",
            "--urgency=normal",
        ),
    ];
    for (kind, category, urgency) in kinds {
        let mut notification = event(1, kind);
        notification.body = Text::new("<b>private & quoted</b>").unwrap();
        let command = linux_banner_command(&notification);
        let args: Vec<_> = command.get_args().map(|a| a.to_str().unwrap()).collect();
        assert!(args.windows(2).any(|a| a == ["--category", category]));
        assert!(args.contains(&urgency));
        assert_eq!(
            args.iter().filter(|a| a.starts_with("--urgency=")).count(),
            1
        );
        assert!(!args.iter().any(|a| a.starts_with("--action")));
        assert!(args.contains(&"--hint=boolean:suppress-sound:true"));
        assert_eq!(
            args.last().copied(),
            Some("&lt;b&gt;private &amp; quoted&lt;/b&gt;")
        );
    }
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "owned notification daemon/session only; system sound capability must be declared"]
fn native_category_fixture() {
    let case = std::env::var("NMT_NOTIFICATION_NATIVE_CASE").expect("explicit native case");
    let suppressed = std::env::var("NMT_NOTIFICATION_NATIVE_SUPPRESS").as_deref() == Ok("1");
    let account = scope(101, 1);
    let kind = match case.as_str() {
        "messages" => message(account, 1),
        "conference" => join(account, 1, false),
        "reminder" => NotificationKind::MessageReminder {
            scope: account,
            conversation: call(account, 1).conversation,
            id: NotificationId(1),
        },
        "incoming" => NotificationKind::IncomingCall {
            call: call(account, 1),
        },
        "calendar" => NotificationKind::Calendar {
            scope: account,
            id: NotificationId(1),
        },
        _ => panic!("unsupported native case"),
    };
    let mut policy = NotificationPreferences::default();
    *delivery_mut(&mut policy, kind.category()) = if suppressed {
        NotificationDelivery::OFF
    } else {
        NotificationDelivery::default()
    };
    let runtime = NotificationRuntime::start(policy).unwrap();
    let mut fixture_sources = sources(account, 1);
    fixture_sources.calendar = SourceAvailability::Available;
    runtime.update_sources(None, fixture_sources).unwrap();
    let mut notification = event(1, kind);
    notification.title = Text::new(&format!("NMT notification category {case}")).unwrap();
    runtime.submit(notification).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    let completion = loop {
        if let Some(completion) = runtime.take_completion() {
            break completion;
        }
        assert!(
            Instant::now() < deadline,
            "notification helper did not finish"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let expected = if suppressed {
        DeliveryOutcome::Suppressed
    } else {
        DeliveryOutcome::SubmittedToSystem
    };
    assert_eq!(completion.banner, expected);
    let sound =
        std::env::var("NMT_NOTIFICATION_NATIVE_SOUND").expect("observed daemon sound capability");
    assert_eq!(
        completion.sound,
        if suppressed {
            DeliveryOutcome::Suppressed
        } else {
            match sound.as_str() {
                "system" => DeliveryOutcome::SubmittedToSystem,
                "unsupported" => DeliveryOutcome::Failed(NotificationError::Unavailable),
                _ => panic!("unsupported sound capability declaration"),
            }
        }
    );
    println!(
        "category={case} suppressed={suppressed} completion={completion:?}; fixture source only, no live account or call action"
    );
    runtime.request_shutdown();
    while runtime.snapshot() != NotificationState::Stopped {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn native_actions_require_current_call_and_real_dispatcher_capability() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Dispatcher(AtomicUsize);
    impl NotificationCallDispatcher for Dispatcher {
        fn available_actions(&self, _: CallScope) -> NativeCallActions {
            NativeCallActions {
                accept: false,
                decline: true,
            }
        }
        fn dispatch(&self, _: CallScope, _: NativeCallAction) -> Result<(), NotificationError> {
            self.0.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }
    let account = scope(42, 1);
    let shared = Shared {
        state: Mutex::new(State::new(NotificationPreferences::default())),
        wake: Condvar::new(),
    };
    let notification = event(
        1,
        NotificationKind::IncomingCall {
            call: call(account, 1),
        },
    );
    {
        let mut state = shared.lock();
        state.update_sources(None, sources(account, 1)).unwrap();
        state.admit(notification.clone(), Instant::now()).unwrap();
        state.next().unwrap();
    }
    let dispatcher = Dispatcher(AtomicUsize::new(0));
    assert_eq!(
        macos::dispatch_action(&shared, &notification, None, NativeCallAction::Accept),
        Err(NotificationError::Unavailable)
    );
    assert_eq!(
        macos::dispatch_action(
            &shared,
            &notification,
            Some(&dispatcher),
            NativeCallAction::Accept
        ),
        Err(NotificationError::Unavailable)
    );
    assert_eq!(dispatcher.0.load(Ordering::Relaxed), 0);
    assert_eq!(
        macos::dispatch_action(
            &shared,
            &notification,
            Some(&dispatcher),
            NativeCallAction::Decline
        ),
        Ok(())
    );
    let mut next = sources(account, 2);
    next.ringing = None;
    shared.lock().update_sources(Some(account), next).unwrap();
    assert_eq!(
        macos::dispatch_action(
            &shared,
            &notification,
            Some(&dispatcher),
            NativeCallAction::Decline
        ),
        Err(NotificationError::Stale)
    );
    assert_eq!(dispatcher.0.load(Ordering::Relaxed), 1);
}

#[test]
fn linux_sound_only_does_not_escape_system_notification_focus_policy() {
    let mut policy = NotificationPreferences::default();
    policy.messages.banners = false;
    let shared = Shared {
        state: Mutex::new(State::new(policy)),
        wake: Condvar::new(),
    };
    let event = test(1, NotificationCategory::Messages);
    shared.lock().admit(event.clone(), Instant::now()).unwrap();
    shared.lock().next().unwrap();
    if cfg!(target_os = "linux") {
        assert_eq!(
            deliver_pair(&shared, &event, None),
            (
                DeliveryOutcome::Suppressed,
                DeliveryOutcome::Failed(NotificationError::Unavailable)
            )
        );
    }
    let command = linux_command(
        &event,
        Some(Path::new("/bundled sounds/message-notification.wav")),
    );
    let args: Vec<_> = command.get_args().map(|s| s.to_str().unwrap()).collect();
    assert!(args.contains(&"--urgency=normal"));
    assert!(args.contains(&"--hint=boolean:suppress-sound:false"));
    assert!(args.contains(&"--hint=string:sound-file:/bundled sounds/message-notification.wav"));
    assert!(!args.iter().any(|s| s.contains("sound-name")));
    assert!(!args.iter().any(|s| s.contains("critical")));
}
