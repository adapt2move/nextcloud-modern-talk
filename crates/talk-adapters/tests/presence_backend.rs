//! Opt-in test against a disposable server with a seeded direct-message status.
#![cfg(feature = "backend-test")]

use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, test_support};
use talk_core::{
    bounded::Text,
    ids::{AccountId, AccountScope, ClockEpoch, HostId, OperationId, RuntimeGeneration},
    lifecycle::Cancellation,
    ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    preferences::{
        RequestId,
        account::{AccountPreferenceChange, StatusChoice, StatusClearAfter, StatusMessageChange},
    },
    protocol::{
        RoomKind, UserPresence,
        conversation::{ConversationCommand, ConversationResult},
    },
};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the disposable presence lab fixture"]
async fn direct_room_list_exposes_server_status() {
    let root = std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture directory");
    let scope = AccountScope {
        account: AccountId::from_local(910),
        runtime: RuntimeGeneration::from_local(1),
    };
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(910));
    let mut client = test_support::from_private_fixture(
        &Path::new(&root).join("alice.json"),
        scope,
        HostId::from_local(910),
        runtime.clone(),
    )
    .expect("private synthetic fixture");
    let expected_room = client.fixture_conversation().expect("seeded direct room");
    let mut deadline = runtime.now();
    deadline.millis += 60_000;
    let context = OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    };
    let ProtocolReply::Conversations(list) = client
        .request(context, ProtocolRequest::ListConversations)
        .await
        .expect("released server room list")
    else {
        panic!("room list reply");
    };
    assert_eq!(list.scope, scope);
    let direct = list
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.id == expected_room)
        .expect("seeded direct room");
    assert_eq!(direct.kind, RoomKind::Direct);
    let status = direct
        .user_status
        .as_ref()
        .expect("enabled user_status capability and status");
    assert_eq!(status.presence, UserPresence::Away);
    assert_eq!(status.icon.as_ref().map(|v| v.as_str()), Some("☕"));
    assert_eq!(
        status.message.as_ref().map(|v| v.as_str()),
        Some("Presence fixture")
    );
    let ProtocolReply::ConversationSettings {
        result: ConversationResult::Members(members),
        ..
    } = client
        .request(
            OperationContext {
                scope,
                deadline,
                cancellation: Cancellation::default(),
            },
            ProtocolRequest::ConversationSettings {
                conversation: expected_room,
                operation: OperationId::from_local(900),
                command: ConversationCommand::Members,
            },
        )
        .await
        .expect("released server member status")
    else {
        panic!("member list reply");
    };
    let bob = members
        .as_slice()
        .iter()
        .find(|member| {
            member
                .user_status
                .as_ref()
                .is_some_and(|status| status.presence == UserPresence::Away)
        })
        .expect("seeded participant");
    let status = bob.user_status.as_ref().expect("participant status");
    assert_eq!(status.presence, UserPresence::Away);
    assert_eq!(status.icon.as_ref().map(|v| v.as_str()), Some("☕"));
    assert_eq!(
        status.message.as_ref().map(|v| v.as_str()),
        Some("Presence fixture")
    );

    let group = list
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.name.as_str() == "Presence group")
        .expect("seeded group")
        .id;
    let fresh_context = || OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    };
    let ProtocolReply::ConversationSettings {
        result: ConversationResult::Members(group_members),
        ..
    } = client
        .request(
            fresh_context(),
            ProtocolRequest::ConversationSettings {
                conversation: group,
                operation: OperationId::from_local(901),
                command: ConversationCommand::Members,
            },
        )
        .await
        .expect("group members")
    else {
        panic!("group members reply")
    };
    let bob = group_members
        .as_slice()
        .iter()
        .find(|member| {
            member
                .user_status
                .as_ref()
                .is_some_and(|status| status.presence == UserPresence::Away)
        })
        .expect("group participant")
        .id;
    let mut stale = bob;
    stale.local ^= 1;
    assert_eq!(
        client
            .request(
                fresh_context(),
                ProtocolRequest::CreateDirectForMember {
                    operation: OperationId::from_local(902),
                    member: stale,
                }
            )
            .await
            .unwrap_err()
            .kind,
        talk_core::ports::ErrorKind::WrongScope
    );
    for _ in 0..2 {
        let ProtocolReply::Conversation(direct) = client
            .request(
                fresh_context(),
                ProtocolRequest::CreateDirectForMember {
                    operation: OperationId::from_local(903),
                    member: bob,
                },
            )
            .await
            .expect("open direct from member")
        else {
            panic!("direct room reply")
        };
        assert_eq!(direct.id, expected_room);
    }
    for (number, choice) in [
        (1, StatusChoice::Away),
        (2, StatusChoice::DoNotDisturb),
        (3, StatusChoice::Invisible),
        (4, StatusChoice::Online),
    ] {
        let mut deadline = runtime.now();
        deadline.millis += 60_000;
        let context = OperationContext {
            scope,
            deadline,
            cancellation: Cancellation::default(),
        };
        let ProtocolReply::Preferences(updated) = client
            .request(
                context,
                ProtocolRequest::SetPreference {
                    request: RequestId(number),
                    change: AccountPreferenceChange::UserPresence(choice),
                },
            )
            .await
            .expect("set and reconcile own status")
        else {
            panic!("preferences reply")
        };
        assert_eq!(updated.values.scope, scope);
        assert_eq!(updated.values.user_presence, Some(choice));
    }
    let mut deadline = runtime.now();
    deadline.millis += 60_000;
    let context = || OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    };
    let ProtocolReply::Preferences(loaded) = client
        .request(
            context(),
            ProtocolRequest::ReadPreferences {
                request: RequestId(20),
            },
        )
        .await
        .expect("status choices")
    else {
        panic!("preferences reply")
    };
    let predefined = loaded
        .values
        .predefined_statuses
        .as_ref()
        .expect("server predefined statuses")
        .as_slice()
        .first()
        .expect("at least one server choice")
        .clone();
    let ProtocolReply::Preferences(changed) = client
        .request(
            context(),
            ProtocolRequest::SetPreference {
                request: RequestId(21),
                change: AccountPreferenceChange::StatusMessage(StatusMessageChange::Predefined {
                    id: predefined.id.clone(),
                    clear_after: StatusClearAfter::OneHour,
                }),
            },
        )
        .await
        .expect("predefined status")
    else {
        panic!("preferences reply")
    };
    let status = changed.values.status_message.expect("predefined message");
    assert_eq!(
        status.message.as_ref().map(|v| v.as_str()),
        Some(predefined.message.as_str())
    );
    assert!(status.clear_at.is_some());
    let ProtocolReply::Preferences(changed) = client
        .request(
            context(),
            ProtocolRequest::SetPreference {
                request: RequestId(22),
                change: AccountPreferenceChange::StatusMessage(StatusMessageChange::Custom {
                    icon: Some(Text::new("☕").unwrap()),
                    message: Text::new("Synthetic coffee").unwrap(),
                    clear_after: StatusClearAfter::ThirtyMinutes,
                }),
            },
        )
        .await
        .expect("custom status")
    else {
        panic!("preferences reply")
    };
    let status = changed.values.status_message.expect("custom message");
    assert_eq!(
        status.message.as_ref().map(|v| v.as_str()),
        Some("Synthetic coffee")
    );
    assert_eq!(status.icon.as_ref().map(|v| v.as_str()), Some("☕"));
    assert!(status.clear_at.is_some());
    let ProtocolReply::Preferences(cleared) = client
        .request(
            context(),
            ProtocolRequest::SetPreference {
                request: RequestId(23),
                change: AccountPreferenceChange::StatusMessage(StatusMessageChange::Clear),
            },
        )
        .await
        .expect("clear status")
    else {
        panic!("preferences reply")
    };
    assert!(cleared.values.status_message.is_none());
    assert!(
        list.rooms
            .as_slice()
            .iter()
            .filter(|room| room.kind != RoomKind::Direct)
            .all(|room| room.user_status.is_none())
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires the disposable presence lab fixture"]
async fn signaling_typing_is_room_scoped_and_respects_privacy() {
    use std::time::{Duration, Instant};
    let root = std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture directory");
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(911));
    let scope = |account| AccountScope {
        account: AccountId::from_local(account),
        runtime: RuntimeGeneration::from_local(1),
    };
    let alice_scope = scope(911);
    let bob_scope = scope(912);
    let mut alice = test_support::from_private_fixture(
        &Path::new(&root).join("alice.json"),
        alice_scope,
        HostId::from_local(911),
        runtime.clone(),
    )
    .unwrap();
    let mut bob = test_support::from_private_fixture(
        &Path::new(&root).join("bob.json"),
        bob_scope,
        HostId::from_local(912),
        runtime.clone(),
    )
    .unwrap();
    let alice_direct = alice.fixture_conversation().unwrap();
    let context = |scope| {
        let mut deadline = runtime.now();
        deadline.millis += 20_000;
        OperationContext {
            scope,
            deadline,
            cancellation: Cancellation::default(),
        }
    };
    let ProtocolReply::Conversations(rooms) = alice
        .request(context(alice_scope), ProtocolRequest::ListConversations)
        .await
        .unwrap()
    else {
        panic!("rooms")
    };
    let alice_room = rooms
        .rooms
        .as_slice()
        .iter()
        .find(|r| r.name.as_str() == "Presence group")
        .unwrap()
        .id;
    let bob_room = bob.fixture_conversation().unwrap();
    for (client, account, room) in [
        (&mut alice, alice_scope, alice_room),
        (&mut bob, bob_scope, bob_room),
    ] {
        client
            .request(context(account), ProtocolRequest::WatchTyping(room))
            .await
            .unwrap();
    }
    let end = Instant::now() + Duration::from_secs(60);
    let mut observed = false;
    let mut retry_alice = Instant::now();
    let mut retry_bob = Instant::now();
    while Instant::now() < end {
        if let ProtocolReply::Typing(state) = bob
            .request(context(bob_scope), ProtocolRequest::PollTyping(bob_room))
            .await
            .unwrap()
            && !state.running
            && Instant::now() >= retry_bob
        {
            let _ = bob
                .request(context(bob_scope), ProtocolRequest::WatchTyping(bob_room))
                .await;
            retry_bob = Instant::now() + Duration::from_secs(2);
        }
        let _ = bob
            .request(
                context(bob_scope),
                ProtocolRequest::TypingActivity {
                    conversation: bob_room,
                    active: true,
                },
            )
            .await;
        if let ProtocolReply::Typing(observation) = alice
            .request(
                context(alice_scope),
                ProtocolRequest::PollTyping(alice_room),
            )
            .await
            .unwrap()
        {
            if observation
                .people
                .as_slice()
                .iter()
                .any(|person| person.name.as_str() == "lab_bob")
            {
                observed = true;
                break;
            }
            if !observation.running && Instant::now() >= retry_alice {
                let _ = alice
                    .request(
                        context(alice_scope),
                        ProtocolRequest::WatchTyping(alice_room),
                    )
                    .await;
                retry_alice = Instant::now() + Duration::from_secs(2);
            }
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    assert!(
        observed,
        "remote typing should arrive from the joined participant"
    );
    let ProtocolReply::Typing(other_room) = alice
        .request(
            context(alice_scope),
            ProtocolRequest::PollTyping(alice_direct),
        )
        .await
        .unwrap()
    else {
        panic!("other-room observation")
    };
    assert!(!other_room.running && other_room.people.as_slice().is_empty());

    // No more activity renewals: the sender's lease must retire on idle.
    let end = Instant::now() + Duration::from_secs(12);
    let mut idle_cleared = false;
    while Instant::now() < end {
        if let ProtocolReply::Typing(observation) = alice
            .request(
                context(alice_scope),
                ProtocolRequest::PollTyping(alice_room),
            )
            .await
            .unwrap()
            && observation.running
            && observation.people.as_slice().is_empty()
        {
            idle_cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(idle_cleared, "idle must clear the indicator");
    let end = Instant::now() + Duration::from_secs(12);
    let mut restarted = false;
    while Instant::now() < end {
        bob.request(
            context(bob_scope),
            ProtocolRequest::TypingActivity {
                conversation: bob_room,
                active: true,
            },
        )
        .await
        .unwrap();
        if let ProtocolReply::Typing(observation) = alice
            .request(
                context(alice_scope),
                ProtocolRequest::PollTyping(alice_room),
            )
            .await
            .unwrap()
            && !observation.people.as_slice().is_empty()
        {
            restarted = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(restarted, "new typing should become visible after idle");

    bob.request(
        context(bob_scope),
        ProtocolRequest::TypingActivity {
            conversation: bob_room,
            active: false,
        },
    )
    .await
    .unwrap();
    let end = Instant::now() + Duration::from_secs(4);
    let mut cleared = false;
    while Instant::now() < end {
        if let ProtocolReply::Typing(observation) = alice
            .request(
                context(alice_scope),
                ProtocolRequest::PollTyping(alice_room),
            )
            .await
            .unwrap()
            && observation.running
            && observation.people.as_slice().is_empty()
        {
            cleared = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(cleared, "stop or expiry clears the indicator");
    // The live HPB socket must resume after a network/suspend hint, rather than
    // waiting for its heartbeat to time out or requiring another room join.
    runtime.network_changed();
    let resumed_at = Instant::now();
    tokio::time::sleep(Duration::from_secs(1)).await;
    let mut resumed = false;
    while resumed_at.elapsed() < Duration::from_secs(15) {
        let _ = bob
            .request(
                context(bob_scope),
                ProtocolRequest::TypingActivity {
                    conversation: bob_room,
                    active: true,
                },
            )
            .await;
        if let Ok(ProtocolReply::Typing(observation)) = alice
            .request(
                context(alice_scope),
                ProtocolRequest::PollTyping(alice_room),
            )
            .await
            && observation.running
            && observation
                .people
                .as_slice()
                .iter()
                .any(|p| p.name.as_str() == "lab_bob")
        {
            resumed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(resumed, "HPB typing must recover after a connectivity hint");
    eprintln!(
        "HPB_NETWORK_RECOVERY_MS={}",
        resumed_at.elapsed().as_millis()
    );

    bob.request(
        context(bob_scope),
        ProtocolRequest::SetPreference {
            request: RequestId(999),
            change: AccountPreferenceChange::ShareTypingStatus(false),
        },
    )
    .await
    .unwrap();
    assert!(
        bob.request(
            context(bob_scope),
            ProtocolRequest::TypingActivity {
                conversation: bob_room,
                active: true
            }
        )
        .await
        .is_err()
    );
    alice
        .request(
            context(alice_scope),
            ProtocolRequest::SetPreference {
                request: RequestId(1000),
                change: AccountPreferenceChange::ShareTypingStatus(false),
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Typing(observation) = alice
        .request(
            context(alice_scope),
            ProtocolRequest::PollTyping(alice_room),
        )
        .await
        .unwrap()
    else {
        panic!("typing observation")
    };
    assert!(!observation.running && observation.people.as_slice().is_empty());
    alice
        .request(context(alice_scope), ProtocolRequest::StopTyping)
        .await
        .unwrap();
    bob.request(context(bob_scope), ProtocolRequest::StopTyping)
        .await
        .unwrap();
}
