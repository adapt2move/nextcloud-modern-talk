//! Opt-in test against a disposable server with a seeded direct-message status.
#![cfg(feature = "backend-test")]

use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, test_support};
use talk_core::{
    ids::{AccountId, AccountScope, ClockEpoch, HostId, RuntimeGeneration},
    lifecycle::Cancellation,
    ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    protocol::{RoomKind, UserPresence},
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
    assert!(
        list.rooms
            .as_slice()
            .iter()
            .filter(|room| room.kind != RoomKind::Direct)
            .all(|room| room.user_status.is_none())
    );
}
