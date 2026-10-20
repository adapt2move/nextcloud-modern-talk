//! Explicitly ignored: requires the disposable real backend and owner-private fixtures.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::diagnostics::{Signal, SignalRef};
use talk_adapters::nextcloud::{NetworkRuntime, NextcloudClient, test_support};
use talk_core::bounded::Text;
use talk_core::ids::{
    AccountId, AccountScope, CallGeneration, CallScope, ClockEpoch, HostId, OperationId,
    RuntimeGeneration,
};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::{Decision, Moment};
use talk_core::ports::{
    ErrorKind, NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest,
};
use talk_core::protocol::{RoomKind, SendMessage, SendState, ServerFamily};

fn scope(n: u128) -> AccountScope {
    AccountScope {
        account: AccountId::from_local(n),
        runtime: RuntimeGeneration::from_local(1),
    }
}
fn context(runtime: &NetworkRuntime, n: u128) -> OperationContext {
    let now = runtime.now();
    OperationContext {
        scope: scope(n),
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis + 3000,
        },
        cancellation: Cancellation::default(),
    }
}
fn client(root: &Path, name: &str, n: u128, runtime: &NetworkRuntime) -> NextcloudClient {
    test_support::from_private_fixture(
        &root.join(name),
        scope(n),
        HostId::from_local(1),
        runtime.clone(),
    )
    .expect("private fixture intake")
}
#[tokio::test]
#[ignore = "real lab only; see docs/backend-lab.md for private fixture setup."]
async fn actual_rust_adapter_against_released_nextcloud() {
    let directory = std::env::var_os("NMT_FIXTURE_DIR")
        .expect("private path required; no secret environment values");
    let root = Path::new(&directory);
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(100));
    let mut alice = client(root, "alice.json", 1, &runtime);
    let mut bob = client(root, "bob.json", 2, &runtime);
    let mut outsider = client(root, "outsider.json", 3, &runtime);
    let mut bad = client(root, "bad.json", 4, &runtime);
    let family = match std::fs::read_to_string(root.join("profile"))
        .unwrap()
        .trim()
    {
        "r34" => ServerFamily::Nextcloud34,
        "r35" => ServerFamily::Nextcloud35,
        _ => panic!("unsupported fixture"),
    };
    let ProtocolReply::Account(account) = alice
        .request(context(&runtime, 1), ProtocolRequest::InspectAccount)
        .await
        .expect("actual authenticated probe")
    else {
        panic!("account reply")
    };
    assert_eq!(account.family, family);
    assert!(account.talk_available && account.reference_correlation);
    assert_eq!(
        account.channel_semantics,
        family == ServerFamily::Nextcloud35
    );
    assert_eq!(
        bad.request(context(&runtime, 4), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ReauthenticationRequired
    );
    assert_eq!(
        bad.request(context(&runtime, 4), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    let aid = alice.fixture_conversation().unwrap();
    let bid = bob.fixture_conversation().unwrap();
    let oid = outsider.fixture_conversation().unwrap();
    assert_ne!(aid, bid);
    assert_eq!(
        outsider
            .request(
                context(&runtime, 3),
                ProtocolRequest::ReconcileConversation(oid)
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::NotFound
    );
    let ProtocolReply::Conversations(list) = alice
        .request(context(&runtime, 1), ProtocolRequest::ListConversations)
        .await
        .expect("room list")
    else {
        panic!("list reply")
    };
    assert!(list.rooms.as_slice().iter().any(|room| room.id == aid
        && room.kind == RoomKind::Group
        && room.actions.send_message == Decision::Allowed));
    if family == ServerFamily::Nextcloud35 {
        assert!(
            list.rooms
                .as_slice()
                .iter()
                .any(|r| r.channel && r.actions.join_call == Decision::Denied)
        );
    }
    let ProtocolReply::Participants(people) = alice
        .request(context(&runtime, 1), ProtocolRequest::Participants(aid))
        .await
        .expect("participant normalization")
    else {
        panic!("participants reply")
    };
    assert!(
        people.people.as_slice().len() >= 2 && people.people.as_slice().iter().any(|p| p.is_self)
    );
    let operation = OperationId::from_local(55);
    let ProtocolReply::Send(receipt) = bob
        .request(
            context(&runtime, 2),
            ProtocolRequest::Send(SendMessage {
                conversation: bid,
                operation,
                text: Text::new("Rust I1 — Grüße, Unicode 🦀 und Zeile\nzwei").unwrap(),
            }),
        )
        .await
        .expect("Rust send request")
    else {
        panic!("send reply")
    };
    let SendState::ServerAccepted(sent) = receipt.state else {
        panic!("server acceptance required, no delivered claim")
    };
    let ProtocolReply::History(history) = alice
        .request(
            context(&runtime, 1),
            ProtocolRequest::History {
                conversation: aid,
                before: None,
            },
        )
        .await
        .expect("member history")
    else {
        panic!("history reply")
    };
    assert!(
        history
            .messages
            .as_slice()
            .iter()
            .any(|m| m.id.sequence == sent.sequence && m.text.as_str().contains("Grüße"))
    );
    let ProtocolReply::History(own) = bob
        .request(
            context(&runtime, 2),
            ProtocolRequest::History {
                conversation: bid,
                before: None,
            },
        )
        .await
        .expect("own history correlation")
    else {
        panic!("history reply")
    };
    assert!(
        own.messages
            .as_slice()
            .iter()
            .any(|m| m.own_operation == Some(operation))
    );
    let cursor = history.older.expect("bounded backwards pagination cursor");
    let ProtocolReply::History(older) = alice
        .request(
            context(&runtime, 1),
            ProtocolRequest::History {
                conversation: aid,
                before: Some(cursor),
            },
        )
        .await
        .expect("history pagination")
    else {
        panic!("history reply")
    };
    assert!(
        older
            .messages
            .as_slice()
            .iter()
            .all(|m| m.id.sequence < cursor.sequence)
    );
    let ProtocolReply::Conversations(member_rooms) = bob
        .request(context(&runtime, 2), ProtocolRequest::ListConversations)
        .await
        .expect("member room list")
    else {
        panic!("list reply")
    };
    let restricted = member_rooms
        .rooms
        .as_slice()
        .iter()
        .find(|r| r.name.as_str() == "Rust I1 read only")
        .expect("restricted room fixture");
    assert_eq!(restricted.actions.send_message, Decision::Denied);
    assert_eq!(
        bob.request(
            context(&runtime, 2),
            ProtocolRequest::Send(SendMessage {
                conversation: restricted.id,
                operation: OperationId::from_local(56),
                text: Text::new("must not post").unwrap()
            })
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::PermissionDenied
    );
    assert!(matches!(
        bob.request(context(&runtime, 2), ProtocolRequest::MarkRead(sent))
            .await
            .expect("read marker"),
        ProtocolReply::ReadAcknowledged(_)
    ));
    assert!(matches!(
        alice
            .request(
                context(&runtime, 1),
                ProtocolRequest::ActivateConversation(aid)
            )
            .await
            .expect("session activation"),
        ProtocolReply::ConversationActive(_)
    ));
    let call = CallScope {
        account: scope(1),
        conversation: aid,
        call: CallGeneration::from_local(9),
    };
    let ProtocolReply::Call(observation) = alice
        .request(context(&runtime, 1), ProtocolRequest::ObserveCall(call))
        .await
        .expect("call/settings observation")
    else {
        panic!("call observation")
    };
    assert_ne!(observation.native_media, Decision::Allowed);
    assert!(matches!(
        alice
            .request(context(&runtime, 1), ProtocolRequest::Leave(call))
            .await
            .expect("own session leave call"),
        ProtocolReply::LeaveAcknowledged(_)
    ));
    alice
        .request(
            context(&runtime, 1),
            ProtocolRequest::DeactivateConversation(aid),
        )
        .await
        .expect("deactivate own conversation session");
    let bcall = CallScope {
        account: scope(2),
        conversation: bid,
        call: CallGeneration::from_local(9),
    };
    alice
        .diagnostic_join_synthetic_call(&context(&runtime, 1), call)
        .await
        .expect("diagnostic membership A");
    bob.diagnostic_join_synthetic_call(&context(&runtime, 2), bcall)
        .await
        .expect("diagnostic membership B");
    let a_peers = alice
        .diagnostic_peers(&context(&runtime, 1), call)
        .await
        .expect("opaque A peer");
    let b_peers = bob
        .diagnostic_peers(&context(&runtime, 2), bcall)
        .await
        .expect("opaque B peer");
    assert_eq!(a_peers.as_slice().len(), 1);
    assert_eq!(b_peers.as_slice().len(), 1);
    let (sender, receiver, sender_n, receiver_n, sender_call, receiver_call, outgoing, incoming) =
        if a_peers.as_slice()[0].should_offer() {
            (
                &mut alice,
                &mut bob,
                1,
                2,
                call,
                bcall,
                &a_peers.as_slice()[0],
                &b_peers.as_slice()[0],
            )
        } else {
            (
                &mut bob,
                &mut alice,
                2,
                1,
                bcall,
                call,
                &b_peers.as_slice()[0],
                &a_peers.as_slice()[0],
            )
        };
    assert!(outgoing.should_offer());
    assert!(!incoming.should_offer());
    sender
        .diagnostic_select_peer(&context(&runtime, sender_n), outgoing)
        .unwrap();
    receiver
        .diagnostic_select_peer(&context(&runtime, receiver_n), incoming)
        .unwrap();
    let peer = sender
        .diagnostic_take_selected_peer(&context(&runtime, sender_n), sender_call)
        .unwrap();
    // Synthetic envelopes only. Follow actual internal-role ordering; do not
    // force a second SID to avoid collisions or retain a guarded response page.
    let sdp = "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\ns=I2 transport fixture\r\nt=0 0\r\n";
    sender
        .diagnostic_send(
            &context(&runtime, sender_n),
            &peer,
            Signal::Offer(Text::new(sdp).unwrap()),
        )
        .await
        .expect("OCS offer processing");
    let mut received_offer = false;
    for _ in 0..3 {
        let page = receiver
            .diagnostic_poll(&context(&runtime, receiver_n), receiver_call)
            .await
            .expect("received offer poll");
        received_offer = page
            .inspect()
            .unwrap()
            .signals
            .iter()
            .any(|s| matches!(s.signal(), SignalRef::Offer(_)));
        drop(page);
        if received_offer {
            break;
        }
    }
    assert!(
        received_offer,
        "actual offer required, never infer receipt from POST"
    );
    let incoming = receiver
        .diagnostic_take_selected_peer(&context(&runtime, receiver_n), receiver_call)
        .unwrap();
    receiver
        .diagnostic_send(
            &context(&runtime, receiver_n),
            &incoming,
            Signal::Answer(Text::new(sdp).unwrap()),
        )
        .await
        .expect("OCS answer processing");
    receiver
        .diagnostic_send(
            &context(&runtime, receiver_n),
            &incoming,
            Signal::Candidate {
                candidate: Text::new("candidate:1 1 udp 1 127.0.0.1 9 typ host").unwrap(),
                mid: Text::new("0").unwrap(),
                index: 0,
            },
        )
        .await
        .expect("OCS candidate processing");
    let (mut got_answer, mut got_candidate) = (false, false);
    for _ in 0..3 {
        let answers = sender
            .diagnostic_poll(&context(&runtime, sender_n), sender_call)
            .await
            .expect("OCS answer/candidate");
        for received in answers.inspect().unwrap().signals {
            assert!(received.peer().same_negotiation(&peer));
            got_answer |= matches!(received.signal(), SignalRef::Answer(_));
            got_candidate |= matches!(received.signal(), SignalRef::Candidate { .. });
        }
        drop(answers);
        if got_answer && got_candidate {
            break;
        }
    }
    assert!(
        got_answer && got_candidate,
        "actual correlated envelopes required, never infer receipt from POST"
    );
    for (client, n, call) in [(&mut alice, 1, call), (&mut bob, 2, bcall)] {
        client
            .request(context(&runtime, n), ProtocolRequest::Leave(call))
            .await
            .expect("diagnostic call leave");
        client
            .request(
                context(&runtime, n),
                ProtocolRequest::DeactivateConversation(call.conversation),
            )
            .await
            .expect("diagnostic session leave");
    }
    let ProtocolReply::Conversation(created) = alice
        .request(
            context(&runtime, 1),
            ProtocolRequest::CreateGroup {
                name: Text::new("Rust I1 created").unwrap(),
            },
        )
        .await
        .expect("create group")
    else {
        panic!("created room reply")
    };
    assert_eq!(created.kind, RoomKind::Group);
    // Last-owner leave restrictions must remain truthful; do not delete the fixture room to pretend leave succeeded.
    if created.actions.leave_conversation == Decision::Allowed {
        alice
            .request(
                context(&runtime, 1),
                ProtocolRequest::LeaveConversation(created.id),
            )
            .await
            .expect("leave created conversation");
    }
    assert!(matches!(
        bob.request(
            context(&runtime, 2),
            ProtocolRequest::LeaveConversation(bid)
        )
        .await
        .expect("member leaves actual conversation"),
        ProtocolReply::ConversationLeft(_)
    ));
    alice.cancellation().cancel();
    assert_eq!(
        alice
            .request(context(&runtime, 1), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert!(matches!(
        bob.request(context(&runtime, 2), ProtocolRequest::ProbeAccount)
            .await
            .expect("independent account survives"),
        ProtocolReply::AccountValid(_)
    ));
    println!(
        "I1 real Rust adapter: account/capabilities/isolation/denial/rooms/participants/paginated-chat/correlation/read/create/leave/session/call-observation and synthetic OCS offer-answer-ICE envelopes passed; media not exercised"
    );
}
