//! One small explicitly invoked disposable-backend check; no shipping credential intake.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, NextcloudClient, test_support};
use talk_core::{
    bounded::Text,
    ids::{AccountId, AccountScope, ClockEpoch, HostId, OperationId, RuntimeGeneration},
    lifecycle::Cancellation,
    policy::Decision,
    ports::{ErrorKind, NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    preferences::{
        RequestId,
        account::{AccountPreferenceChange, AccountPreferences, NextcloudFolder},
    },
    protocol::{SendMessage, SendState},
};
fn context(runtime: &NetworkRuntime, scope: AccountScope) -> OperationContext {
    let mut deadline = runtime.now();
    deadline.millis += 3000;
    OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    }
}
async fn read(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    scope: AccountScope,
    request: u64,
) -> AccountPreferences {
    let ProtocolReply::Preferences(reply) = client
        .request(
            context(runtime, scope),
            ProtocolRequest::ReadPreferences {
                request: RequestId(request),
            },
        )
        .await
        .unwrap()
    else {
        panic!("preferences reply");
    };
    assert_eq!(reply.request, RequestId(request));
    assert_eq!(reply.values.scope, scope);
    reply.values
}
async fn change(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    scope: AccountScope,
    request: u64,
    change: AccountPreferenceChange,
) -> AccountPreferences {
    let ProtocolReply::Preferences(reply) = client
        .request(
            context(runtime, scope),
            ProtocolRequest::SetPreference {
                request: RequestId(request),
                change,
            },
        )
        .await
        .unwrap()
    else {
        panic!("preferences reply");
    };
    assert_eq!(reply.request, RequestId(request));
    assert_eq!(reply.values.scope, scope);
    reply.values
}
#[tokio::test(flavor = "current_thread")]
#[ignore = "real lab only; two private synthetic account fixtures; no production login"]
async fn two_account_preferences_and_archive_read_change_readback() {
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(101));
    let alice_scope = AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(7),
    };
    let bob_scope = AccountScope {
        account: AccountId::from_local(2),
        runtime: RuntimeGeneration::from_local(8),
    };
    let mut alice = test_support::from_private_fixture(
        Path::new(&std::env::var("NMT_SETTINGS_ALICE_FIXTURE").unwrap()),
        alice_scope,
        HostId::from_local(1),
        runtime.clone(),
    )
    .unwrap();
    let mut bob = test_support::from_private_fixture(
        Path::new(&std::env::var("NMT_SETTINGS_BOB_FIXTURE").unwrap()),
        bob_scope,
        HostId::from_local(1),
        runtime.clone(),
    )
    .unwrap();
    let original_a = read(&mut alice, &runtime, alice_scope, 1).await;
    let original_b = read(&mut bob, &runtime, bob_scope, 1).await;
    assert_eq!(original_a.send_read_receipts, Some(true));
    assert_eq!(original_a.share_typing_status, Some(true));
    assert_eq!(original_a.attachments_allowed, Some(true));
    assert!(original_a.attachment_folder.is_some());
    let private = change(
        &mut alice,
        &runtime,
        alice_scope,
        2,
        AccountPreferenceChange::SendReadReceipts(false),
    )
    .await;
    assert!(!private.show_read_indicators());
    let private = change(
        &mut alice,
        &runtime,
        alice_scope,
        3,
        AccountPreferenceChange::ShareTypingStatus(false),
    )
    .await;
    assert!(!private.send_typing());
    assert!(!private.show_typing_indicators());
    assert_eq!(read(&mut bob, &runtime, bob_scope, 2).await, original_b);
    let folder_a = NextcloudFolder::new("/SettingsProbe/Alice").unwrap();
    let folder_b = NextcloudFolder::new("/SettingsProbe/Bob").unwrap();
    change(
        &mut alice,
        &runtime,
        alice_scope,
        4,
        AccountPreferenceChange::AttachmentFolder(folder_a.clone()),
    )
    .await;
    change(
        &mut bob,
        &runtime,
        bob_scope,
        3,
        AccountPreferenceChange::AttachmentFolder(folder_b.clone()),
    )
    .await;
    assert_eq!(
        read(&mut alice, &runtime, alice_scope, 5)
            .await
            .attachment_folder,
        Some(folder_a.clone())
    );
    assert_eq!(
        read(&mut bob, &runtime, bob_scope, 4)
            .await
            .attachment_folder,
        Some(folder_b)
    );
    // Prepared as a regular file by the disposable lab. The server must reject it.
    let rejected = alice
        .request(
            context(&runtime, alice_scope),
            ProtocolRequest::SetPreference {
                request: RequestId(6),
                change: AccountPreferenceChange::AttachmentFolder(
                    NextcloudFolder::new("/SettingsProbe/regular.txt").unwrap(),
                ),
            },
        )
        .await;
    assert!(rejected.is_err());
    assert_eq!(
        read(&mut alice, &runtime, alice_scope, 7)
            .await
            .attachment_folder,
        Some(folder_a)
    );
    assert!(NextcloudFolder::new("/../escape").is_err());
    // Privacy is never a text-sending lock.
    let ProtocolReply::Conversation(room) = alice
        .request(
            context(&runtime, alice_scope),
            ProtocolRequest::CreateGroup {
                name: Text::new("Settings archive probe").unwrap(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("room reply");
    };
    assert_eq!(room.archived, Some(false));
    assert_eq!(room.archive, Decision::Allowed);
    assert_eq!(room.actions.send_message, Decision::Allowed);
    let ProtocolReply::Send(sent) = alice
        .request(
            context(&runtime, alice_scope),
            ProtocolRequest::Send(SendMessage {
                conversation: room.id,
                operation: OperationId::from_local(71),
                text: Text::new("Private receipts still permit chat").unwrap(),
            }),
        )
        .await
        .unwrap()
    else {
        panic!("send reply");
    };
    assert!(matches!(sent.state, SendState::ServerAccepted(_)));
    let ProtocolReply::History(history) = alice
        .request(
            context(&runtime, alice_scope),
            ProtocolRequest::History {
                conversation: room.id,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("history reply");
    };
    assert!(history.common_read.is_none());
    for archived in [true, false] {
        let ProtocolReply::Conversation(changed) = alice
            .request(
                context(&runtime, alice_scope),
                ProtocolRequest::SetArchived {
                    conversation: room.id,
                    archived,
                },
            )
            .await
            .unwrap()
        else {
            panic!("room reply");
        };
        assert_eq!(changed.archived, Some(archived));
        let ProtocolReply::Conversations(list) = alice
            .request(
                context(&runtime, alice_scope),
                ProtocolRequest::ListConversations,
            )
            .await
            .unwrap()
        else {
            panic!("list reply");
        };
        assert_eq!(list.scope, alice_scope);
        assert_eq!(
            list.rooms
                .as_slice()
                .iter()
                .find(|value| value.id == room.id)
                .unwrap()
                .archived,
            Some(archived)
        );
    }
    let wrong_scope = alice
        .request(
            context(&runtime, bob_scope),
            ProtocolRequest::SetArchived {
                conversation: room.id,
                archived: true,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(wrong_scope.kind, ErrorKind::WrongScope);
    let stale_scope = AccountScope {
        runtime: RuntimeGeneration::from_local(99),
        ..alice_scope
    };
    assert_eq!(
        alice
            .request(
                context(&runtime, stale_scope),
                ProtocolRequest::ReadPreferences {
                    request: RequestId(8)
                }
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    change(
        &mut alice,
        &runtime,
        alice_scope,
        9,
        AccountPreferenceChange::SendReadReceipts(true),
    )
    .await;
    change(
        &mut alice,
        &runtime,
        alice_scope,
        10,
        AccountPreferenceChange::ShareTypingStatus(true),
    )
    .await;
    change(
        &mut alice,
        &runtime,
        alice_scope,
        11,
        AccountPreferenceChange::AttachmentFolder(original_a.attachment_folder.clone().unwrap()),
    )
    .await;
    change(
        &mut bob,
        &runtime,
        bob_scope,
        5,
        AccountPreferenceChange::AttachmentFolder(original_b.attachment_folder.clone().unwrap()),
    )
    .await;
    assert_eq!(
        read(&mut alice, &runtime, alice_scope, 12).await,
        original_a
    );
    assert_eq!(read(&mut bob, &runtime, bob_scope, 6).await, original_b);
    alice
        .request(
            context(&runtime, alice_scope),
            ProtocolRequest::LeaveConversation(room.id),
        )
        .await
        .unwrap();
}
