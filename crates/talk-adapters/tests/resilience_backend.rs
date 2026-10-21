//! Opt-in recovery checks through the disposable lab's bounded fault relay.
#![cfg(feature = "backend-test")]

use std::path::PathBuf;
use talk_adapters::nextcloud::{NetworkRuntime, test_support};
use talk_core::bounded::Text;
use talk_core::ids::{AccountId, AccountScope, ClockEpoch, HostId, OperationId, RuntimeGeneration};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::Moment;
use talk_core::ports::{
    ErrorKind, NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest,
};
use talk_core::protocol::SendState;
use talk_core::protocol::messaging::{ComposeOptions, MessagingCommand, MessagingResult};

#[tokio::test]
#[ignore = "requires the private tests/lab/resilience.py fixture"]
async fn history_recovers_after_gateway_failures_without_losing_the_session() {
    let directory = PathBuf::from(std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture"));
    let scope = AccountScope {
        account: AccountId::from_local(701),
        runtime: RuntimeGeneration::from_local(1),
    };
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(701));
    let context = || {
        let now = runtime.now();
        OperationContext {
            scope,
            cancellation: Cancellation::default(),
            deadline: Moment {
                epoch: now.epoch,
                millis: now.millis + 60_000,
            },
        }
    };
    let mut client = test_support::from_private_fixture(
        &directory.join("alice.json"),
        scope,
        HostId::from_local(701),
        runtime.clone(),
    )
    .expect("private synthetic account");
    client
        .request(context(), ProtocolRequest::InspectAccount)
        .await
        .unwrap();
    let room = client.fixture_conversation().unwrap();
    client
        .request(context(), ProtocolRequest::ActivateConversation(room))
        .await
        .unwrap();
    let mut accepted = Vec::new();
    for (index, (mode, expected)) in [
        ("healthy", None),
        ("500", Some(ErrorKind::ServerUnavailable)),
        ("403", Some(ErrorKind::PermissionDenied)),
        ("offline", Some(ErrorKind::Transport)),
    ]
    .into_iter()
    .enumerate()
    {
        let history = |operation| ProtocolRequest::Messaging {
            conversation: room,
            operation: OperationId::from_local(operation),
            command: MessagingCommand::History {
                before: None,
                thread: None,
            },
        };
        if let Some(expected) = expected {
            // Atomic publication keeps the relay from observing a partial mode.
            std::fs::write(directory.join("fault.next"), mode).unwrap();
            std::fs::rename(directory.join("fault.next"), directory.join("fault.mode")).unwrap();
            let failure = client
                .request(context(), history(100 + index as u128))
                .await
                .unwrap_err();
            std::fs::remove_file(directory.join("fault.mode")).unwrap();
            assert_eq!(failure.kind, expected, "{mode} read classification");
            assert_eq!(failure.account, Some(scope.account));
            // A failed chat read must not poison the authenticated binding.
            client
                .request(context(), ProtocolRequest::ProbeAccount)
                .await
                .unwrap();
        }
        let reply = client
            .request(
                context(),
                ProtocolRequest::Messaging {
                    conversation: room,
                    operation: OperationId::from_local(200 + index as u128),
                    command: MessagingCommand::Send {
                        text: Text::new(&format!("Recovery fixture {index}: Grüße 👋")).unwrap(),
                        options: ComposeOptions::default(),
                    },
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Messaging {
            result: MessagingResult::Send(receipt),
            ..
        } = reply
        else {
            panic!("typed send receipt");
        };
        let SendState::ServerAccepted(message) = receipt.state else {
            panic!("server acceptance");
        };
        accepted.push(message);
        let reply = client
            .request(context(), history(300 + index as u128))
            .await
            .unwrap();
        let ProtocolReply::Messaging {
            result: MessagingResult::Messages { messages, .. },
            ..
        } = reply
        else {
            panic!("typed history");
        };
        for message in &accepted {
            assert_eq!(
                messages
                    .as_slice()
                    .iter()
                    .filter(|row| row.message.id == *message)
                    .count(),
                1,
                "accepted messages remain present exactly once after {mode}"
            );
        }
        println!(
            "resilience-backend {mode}: recovery, authenticated send and retained history pass"
        );
    }
}
