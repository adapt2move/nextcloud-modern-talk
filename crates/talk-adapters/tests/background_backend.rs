//! One opt-in finite sequence on the assigned pinned lab. No provisioning or
//! credentials are embedded here; both fixtures use the existing private intake.
#![cfg(feature = "backend-test")]
use std::{path::Path, time::Duration};
use talk_adapters::{
    media::MediaService,
    nextcloud::{NetworkRuntime, NextcloudClient, test_support},
};
use talk_core::{
    bounded::Text,
    calls::{Command, Entry, MediaWishes, Phase},
    ids::{
        AccountId, AccountScope, ClockEpoch, ConversationId, HostId, OperationId, RuntimeGeneration,
    },
    lifecycle::Cancellation,
    ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    protocol::{
        AccountLiveState, ConversationList, SendMessage, SendState,
        conversation::{
            ConversationCommand as C, ConversationResult as R, NotificationLevel, RecipientSource,
        },
    },
};

fn scope(id: u128) -> AccountScope {
    AccountScope {
        account: AccountId::from_local(id),
        runtime: RuntimeGeneration::from_local(1),
    }
}
fn context(runtime: &NetworkRuntime, scope: AccountScope) -> OperationContext {
    let mut deadline = runtime.now();
    deadline.millis += 3000;
    OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    }
}
async fn request(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
    s: AccountScope,
    q: ProtocolRequest,
) -> ProtocolReply {
    c.request(context(r, s), q).await.unwrap()
}
async fn rooms(c: &mut NextcloudClient, r: &NetworkRuntime, s: AccountScope) -> ConversationList {
    let ProtocolReply::Conversations(value) =
        request(c, r, s, ProtocolRequest::ListConversations).await
    else {
        panic!("full room list");
    };
    value
}
async fn setting(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
    s: AccountScope,
    room: ConversationId,
    operation: u128,
    command: C,
) -> R {
    let ProtocolReply::ConversationSettings { result, .. } = request(
        c,
        r,
        s,
        ProtocolRequest::ConversationSettings {
            conversation: room,
            operation: OperationId::from_local(operation),
            command,
        },
    )
    .await
    else {
        panic!("room settings");
    };
    result
}
async fn messages(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
    s: AccountScope,
) -> talk_core::protocol::MessageNotifications {
    let ProtocolReply::MessageNotifications(value) =
        request(c, r, s, ProtocolRequest::PollMessageNotifications).await
    else {
        panic!("account notifications");
    };
    value
}

async fn internal_live(c: &mut NextcloudClient, r: &NetworkRuntime, s: AccountScope) {
    let ProtocolReply::LiveEvents(value) = request(c, r, s, ProtocolRequest::PollLiveEvents).await
    else {
        panic!("typed account live drain");
    };
    assert_eq!(value.scope, s);
    assert_eq!(
        value.state,
        AccountLiveState::Internal,
        "internal lab profile"
    );
    assert!(value.first_transport_failure.is_none());
    assert!(value.changes.as_slice().is_empty());
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "assigned pinned lab only; two owner-private direct-room fixtures, no production login"]
async fn background_message_remote_membership_and_incoming_invitation() {
    let r = NetworkRuntime::new(ClockEpoch::from_local(801));
    let a = scope(801);
    let b = scope(802);
    let fixture = |key: &str, scope| {
        test_support::from_private_fixture(
            Path::new(&std::env::var(key).unwrap()),
            scope,
            HostId::from_local(1),
            r.clone(),
        )
        .unwrap()
    };
    let mut alice = fixture("NMT_BACKGROUND_ALICE_FIXTURE", a);
    let mut bob = fixture("NMT_BACKGROUND_BOB_FIXTURE", b);
    let alice_room = alice
        .fixture_conversation()
        .expect("same synthetic direct room in both fixtures");
    let bob_room = bob
        .fixture_conversation()
        .expect("same synthetic direct room in both fixtures");
    alice.fixture_media(MediaService::new());
    bob.fixture_media(MediaService::new());
    setting(
        &mut alice,
        &r,
        a,
        alice_room,
        1,
        C::SetNotifications(NotificationLevel::Always),
    )
    .await;
    setting(
        &mut alice,
        &r,
        a,
        alice_room,
        2,
        C::SetCallNotifications(true),
    )
    .await;
    let initial = rooms(&mut alice, &r, a).await;
    let unread_before = initial
        .rooms
        .as_slice()
        .iter()
        .find(|r| r.id == alice_room)
        .unwrap()
        .unread_messages
        .unwrap();
    internal_live(&mut alice, &r, a).await;
    assert!(
        messages(&mut alice, &r, a)
            .await
            .notices
            .as_slice()
            .is_empty(),
        "baseline"
    );

    // Alice has never activated/selected this room. Ordinary sending stays on its
    // existing owner; only the account notification source discovers this arrival.
    let ProtocolReply::Send(sent) = request(
        &mut bob,
        &r,
        b,
        ProtocolRequest::Send(SendMessage {
            conversation: bob_room,
            operation: OperationId::from_local(10),
            text: Text::new("Synthetic background arrival").unwrap(),
        }),
    )
    .await
    else {
        panic!("send");
    };
    let SendState::ServerAccepted(sent) = sent.state else {
        panic!("accepted send");
    };
    let mut found = false;
    for attempt in 0..3 {
        internal_live(&mut alice, &r, a).await;
        let batch = messages(&mut alice, &r, a).await;
        println!(
            "background-backend: observation {attempt}, changed={}, notices={}",
            batch.changed,
            batch.notices.as_slice().len()
        );
        if batch
            .notices
            .as_slice()
            .iter()
            .any(|n| n.conversation == alice_room && n.message == sent.sequence)
        {
            found = true;
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    assert!(found, "bounded account notification discovery");
    assert!(
        messages(&mut alice, &r, a)
            .await
            .notices
            .as_slice()
            .is_empty(),
        "no replay"
    );
    let after = rooms(&mut alice, &r, a).await;
    let unread_after = after
        .rooms
        .as_slice()
        .iter()
        .find(|r| r.id == alice_room)
        .unwrap()
        .unread_messages
        .unwrap();
    assert!(
        unread_after > unread_before,
        "observation must not mark read"
    );
    println!("background-backend: nonselected message, replay and unread checks passed");

    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let name = Text::new(&format!("Background fixture {suffix}")).unwrap();
    let ProtocolReply::Conversation(group) = request(
        &mut bob,
        &r,
        b,
        ProtocolRequest::CreateGroup { name: name.clone() },
    )
    .await
    else {
        panic!("remote create");
    };
    let query = std::env::var("NMT_BACKGROUND_ALICE_QUERY").unwrap_or_else(|_| "lab_alice".into());
    let R::Directory(recipients) = setting(
        &mut bob,
        &r,
        b,
        group.id,
        20,
        C::Directory {
            search: Text::new(&query).unwrap(),
            source: RecipientSource::User,
        },
    )
    .await
    else {
        panic!("synthetic directory");
    };
    assert_eq!(recipients.as_slice().len(), 1, "exact synthetic recipient");
    setting(
        &mut bob,
        &r,
        b,
        group.id,
        21,
        C::Invite(recipients.as_slice()[0].id),
    )
    .await;
    let reconciled = rooms(&mut alice, &r, a).await;
    let added = reconciled
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.name == name)
        .expect("remote-created room");
    let added_id = added.id;
    assert_eq!(
        rooms(&mut alice, &r, a)
            .await
            .rooms
            .as_slice()
            .iter()
            .find(|room| room.name == name)
            .unwrap()
            .id,
        added_id
    );
    setting(&mut bob, &r, b, group.id, 22, C::Delete).await;
    let removed = rooms(&mut alice, &r, a).await;
    assert!(
        !removed
            .rooms
            .as_slice()
            .iter()
            .any(|room| room.id == added_id)
    );
    assert!(
        removed
            .rooms
            .as_slice()
            .iter()
            .any(|room| room.id == alice_room)
    );
    println!("background-backend: remote creation, stable identity and confirmed removal passed");

    // The caller is the second synthetic client. Neither side has a device owner
    // or capture wishes; this sequence proves invitation discovery/dismissal only.
    let caller_rooms = rooms(&mut bob, &r, b).await;
    let caller = caller_rooms
        .rooms
        .as_slice()
        .iter()
        .find(|room| room.id == bob_room)
        .unwrap()
        .call_scope
        .unwrap();
    let ProtocolReply::Meeting(started) = request(
        &mut bob,
        &r,
        b,
        ProtocolRequest::Meeting(Command::Start {
            call: caller,
            entry: Entry::OutgoingDirect,
            wishes: MediaWishes::default(),
        }),
    )
    .await
    else {
        panic!("caller start");
    };
    assert!(
        !matches!(started.phase, Phase::Ended | Phase::Waiting(_)),
        "caller admitted"
    );
    let mut invitation = None;
    for attempt in 0..3 {
        let ProtocolReply::IncomingCalls(value) =
            request(&mut alice, &r, a, ProtocolRequest::PollIncomingCalls).await
        else {
            panic!("incoming calls");
        };
        assert_eq!(value.scope, a);
        if let Some(call) = value.call {
            invitation = Some(call);
            break;
        }
        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }
    let invitation = invitation.expect("incoming invitation on observing account");
    assert_eq!(invitation.call.conversation, alice_room);
    assert_eq!(invitation.phase, Phase::Ringing);
    request(
        &mut alice,
        &r,
        a,
        ProtocolRequest::Meeting(Command::Stop(invitation.call)),
    )
    .await;
    let ProtocolReply::IncomingCalls(dismissed) =
        request(&mut alice, &r, a, ProtocolRequest::PollIncomingCalls).await
    else {
        panic!("dismissed invitation");
    };
    assert!(dismissed.call.is_none());
    let ProtocolReply::Meeting(caller) = request(
        &mut bob,
        &r,
        b,
        ProtocolRequest::Meeting(Command::Observe(started.call)),
    )
    .await
    else {
        panic!("caller observation");
    };
    assert_ne!(
        caller.phase,
        Phase::Ended,
        "decline cannot end the caller's account call"
    );
    request(
        &mut bob,
        &r,
        b,
        ProtocolRequest::Meeting(Command::Stop(caller.call)),
    )
    .await;
    alice.cancellation().cancel();
    bob.cancellation().cancel();
    println!(
        "background-backend: incoming discovery and exact invitation decline passed; no media claim"
    );
}
