//! Opt-in public-room regression against each pinned released backend.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, test_support};
use talk_core::{
    bounded::Text,
    ids::{AccountId, AccountScope, ClockEpoch, HostId, OperationId, RuntimeGeneration},
    lifecycle::Cancellation,
    policy::Decision,
    ports::{ErrorKind, NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
    protocol::messaging::{ComposeOptions, MessagingCommand, MessagingResult},
};

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires tests/lab/public_room.py"]
async fn public_room_admission_read_write_and_leave() {
    let root = std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture directory");
    let scope = AccountScope {
        account: AccountId::from_local(940),
        runtime: RuntimeGeneration::from_local(1),
    };
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(940));
    let context = || {
        let mut deadline = runtime.now();
        deadline.millis += 60_000;
        OperationContext {
            scope,
            deadline,
            cancellation: Cancellation::default(),
        }
    };
    for variant in ["public", "password", "lobby", "listed"] {
        let mut client = test_support::from_private_fixture(
            &Path::new(&root).join(format!("{variant}.json")),
            scope,
            HostId::from_local(940),
            runtime.clone(),
        )
        .unwrap();
        let id = client.fixture_conversation().unwrap();
        let open = |password| ProtocolRequest::OpenConversation {
            conversation: id,
            password,
        };
        if variant == "password" {
            for password in [None, Some(Text::new("incorrect").unwrap())] {
                assert_eq!(
                    client
                        .request(context(), open(password))
                        .await
                        .unwrap_err()
                        .kind,
                    ErrorKind::PasswordRequired
                );
            }
        }
        let result = if variant == "listed" {
            let ProtocolReply::OpenConversations(found) = client
                .request(
                    context(),
                    ProtocolRequest::ListOpenConversations {
                        query: Text::new("Public access listed").unwrap(),
                    },
                )
                .await
                .unwrap()
            else {
                panic!("listing")
            };
            assert!(found.rooms.as_slice().iter().any(|r| r.id == id));
            client
                .request(context(), ProtocolRequest::JoinOpenConversation(id))
                .await
        } else {
            client
                .request(
                    context(),
                    open(
                        (variant == "password")
                            .then(|| Text::new("Synthetic room password").unwrap()),
                    ),
                )
                .await
        };
        if variant == "lobby" {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Lobby);
            continue;
        }
        let ProtocolReply::Conversation(room) = result.expect("room admitted") else {
            panic!("room")
        };
        assert_eq!(room.actions.read_messages, Decision::Allowed);
        assert_eq!(room.actions.send_message, Decision::Allowed);
        assert!(
            room.call_scope.is_none(),
            "chat entry grants no call authority"
        );
        let ProtocolReply::Messaging {
            result: MessagingResult::Messages { messages, .. },
            ..
        } = client
            .request(
                context(),
                ProtocolRequest::Messaging {
                    conversation: id,
                    operation: OperationId::from_local(1),
                    command: MessagingCommand::History {
                        before: None,
                        thread: None,
                    },
                },
            )
            .await
            .expect("history after joining")
        else {
            panic!("history")
        };
        assert!(
            messages
                .as_slice()
                .iter()
                .any(|m| m.message.text.as_str() == "Public room history sentinel")
        );
        client
            .request(
                context(),
                ProtocolRequest::Messaging {
                    conversation: id,
                    operation: OperationId::from_local(2),
                    command: MessagingCommand::Send {
                        text: Text::new("Public room member reply").unwrap(),
                        options: ComposeOptions::default(),
                    },
                },
            )
            .await
            .expect("send after joining");
        let ProtocolReply::Messaging {
            result: MessagingResult::Messages { messages, .. },
            ..
        } = client
            .request(
                context(),
                ProtocolRequest::Messaging {
                    conversation: id,
                    operation: OperationId::from_local(3),
                    command: MessagingCommand::History {
                        before: None,
                        thread: None,
                    },
                },
            )
            .await
            .expect("canonical history after sending")
        else {
            panic!("history")
        };
        assert!(
            messages
                .as_slice()
                .iter()
                .any(|message| message.message.text.as_str() == "Public room member reply")
        );
        assert_eq!(
            client
                .request(context(), ProtocolRequest::LeaveConversation(id))
                .await
                .unwrap(),
            ProtocolReply::ConversationLeft(id)
        );
        let ProtocolReply::Conversations(left) = client
            .request(context(), ProtocolRequest::ListConversations)
            .await
            .expect("membership after leaving")
        else {
            panic!("room list")
        };
        assert!(
            !left
                .rooms
                .as_slice()
                .iter()
                .any(|room| room.name.as_str() == format!("Public access {variant}"))
        );
        println!("public-room {variant}: read, write, leave pass");
    }
}
