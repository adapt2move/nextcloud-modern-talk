//! Opt-in account-owner check against an owned 200-room server fixture.
#![cfg(feature = "backend-test")]
use talk_adapters::identity::{IdentityRuntime, RuntimeCommand, RuntimeConfig, RuntimeReply, lab};
use talk_core::ports::{ProtocolReply, ProtocolRequest};

#[tokio::test]
#[ignore = "requires an owned 200-room lab and private NMT_FIXTURE_DIR"]
async fn durable_account_owner_reads_a_complete_two_hundred_room_list() {
    let directory =
        std::path::PathBuf::from(std::env::var_os("NMT_FIXTURE_DIR").expect("owned fixture"));
    let profile = directory.join("room-list-profile");
    lab::seed_private_lab(&profile, &directory.join("alice.json")).expect("seed owned account");
    let owner = IdentityRuntime::start(
        RuntimeConfig::isolated_fixture(profile)
            .with_media(talk_adapters::media::MediaService::new()),
    )
    .expect("start owner");
    let RuntimeReply::Accounts(accounts) = owner
        .try_submit(RuntimeCommand::Accounts)
        .unwrap()
        .wait()
        .await
        .unwrap()
    else {
        panic!("account list")
    };
    assert_eq!(accounts.as_slice().len(), 1);
    let RuntimeReply::Active { summary, .. } = owner
        .try_submit(RuntimeCommand::Restore {
            account: accounts.as_slice()[0].account,
        })
        .unwrap()
        .wait()
        .await
        .unwrap()
    else {
        panic!("restored account")
    };
    let scope = summary.scope.expect("active scope");
    let reply = owner
        .try_submit(RuntimeCommand::Request {
            scope,
            request: ProtocolRequest::ListConversations,
        })
        .unwrap()
        .wait()
        .await
        .expect("complete room list through durable owner");
    let RuntimeReply::Protocol(ProtocolReply::Conversations(list)) = reply else {
        panic!("room list")
    };
    assert_eq!(list.scope, scope);
    assert_eq!(list.rooms.as_slice().len(), 200);
    owner.shutdown();
}
