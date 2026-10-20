//! Finite content checks against owner-private, disposable released Talk fixtures.
#![cfg(feature = "backend-test")]
use std::path::Path;
use talk_adapters::nextcloud::{NetworkRuntime, NextcloudClient, test_support};
use talk_core::bounded::{Page, Text};
use talk_core::ids::{
    AccountId, AccountScope, ClockEpoch, ConversationId, HostId, OperationId, RuntimeGeneration,
};
use talk_core::lifecycle::Cancellation;
use talk_core::policy::{Decision, Moment};
use talk_core::ports::{
    ErrorKind, NextcloudPort, OperationContext, PortError, ProtocolReply, ProtocolRequest,
};
use talk_core::protocol::content::{
    AssetRequest, ContentCommand as C, ContentResult as R, PollCommand as P, PollDraft,
    TransferProgress,
};
use talk_core::protocol::conversation::ConversationCommand;
use talk_core::protocol::messaging::{MessagingCommand as M, MessagingResult as MR};

fn scope(account: u128) -> AccountScope {
    AccountScope {
        account: AccountId::from_local(account),
        runtime: RuntimeGeneration::from_local(1),
    }
}
fn context(runtime: &NetworkRuntime, scope: AccountScope) -> OperationContext {
    let now = runtime.now();
    OperationContext {
        scope,
        cancellation: Cancellation::default(),
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis + 60_000,
        },
    }
}
fn text<const N: usize>(value: &str) -> Text<N> {
    Text::new(value).unwrap()
}
fn page<T, const N: usize>(values: impl IntoIterator<Item = T>) -> Page<T, N> {
    let mut page = Page::empty();
    for value in values {
        page.try_push(value).ok().unwrap();
    }
    page
}
async fn content(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    scope: AccountScope,
    sequence: &mut u128,
    command: C,
) -> Result<R, PortError> {
    *sequence += 1;
    let operation = OperationId::from_local(*sequence);
    match client
        .request(
            context(runtime, scope),
            ProtocolRequest::Content { operation, command },
        )
        .await?
    {
        ProtocolReply::Content {
            operation: actual,
            result,
        } => {
            assert_eq!(actual, operation);
            Ok(result)
        }
        _ => panic!("content reply correlation"),
    }
}
async fn message(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    scope: AccountScope,
    sequence: &mut u128,
    conversation: ConversationId,
    command: M,
) -> Result<MR, PortError> {
    *sequence += 1;
    match client
        .request(
            context(runtime, scope),
            ProtocolRequest::Messaging {
                conversation,
                operation: OperationId::from_local(*sequence),
                command,
            },
        )
        .await?
    {
        ProtocolReply::Messaging { result, .. } => Ok(result),
        _ => panic!("messaging reply"),
    }
}
fn poll(conversation: ConversationId, command: P) -> C {
    C::Poll {
        conversation,
        command,
    }
}
fn draft(hidden: bool) -> PollDraft {
    PollDraft {
        question: text(if hidden {
            "Hidden content fixture"
        } else {
            "Public content fixture"
        }),
        options: page([text("One"), text("Two"), text("Three")]),
        multiple: true,
        hidden_results: hidden,
    }
}
fn raster(result: R) {
    let R::Image(bytes) = result else {
        panic!("raster response");
    };
    let b = bytes.as_slice();
    assert!(
        b.starts_with(b"\x89PNG\r\n\x1a\n") || b.starts_with(b"\xff\xd8") || b.starts_with(b"RIFF")
    );
    assert!(b.len() <= 4 * 1024 * 1024);
}

#[tokio::test]
#[ignore = "requires private disposable NC34/Talk24 or NC35/Talk25 fixtures"]
async fn released_server_polls_avatars_previews_and_downloads() {
    let directory = std::env::var_os("NMT_FIXTURE_DIR").expect("private fixture");
    let root = Path::new(&directory);
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(730));
    let a = scope(730);
    let b = scope(731);
    let mut alice = test_support::from_private_fixture(
        &root.join("alice.json"),
        a,
        HostId::from_local(730),
        runtime.clone(),
    )
    .unwrap();
    let mut bob = test_support::from_private_fixture(
        &root.join("bob.json"),
        b,
        HostId::from_local(731),
        runtime.clone(),
    )
    .unwrap();
    alice
        .request(context(&runtime, a), ProtocolRequest::InspectAccount)
        .await
        .unwrap();
    bob.request(context(&runtime, b), ProtocolRequest::InspectAccount)
        .await
        .unwrap();
    let room = alice.fixture_conversation().unwrap();
    let bob_room = bob.fixture_conversation().unwrap();
    let mut sequence = 1000;
    let R::SelfProfile { name, .. } =
        content(&mut alice, &runtime, a, &mut sequence, C::SelfProfile)
            .await
            .unwrap()
    else {
        panic!("profile");
    };
    assert!(!name.unwrap().as_str().is_empty());
    raster(
        content(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            C::Asset(AssetRequest::SelfAvatar),
        )
        .await
        .unwrap(),
    );
    println!("content-backend profile-and-server-avatar: pass");
    sequence += 1;
    alice
        .request(
            context(&runtime, a),
            ProtocolRequest::ConversationSettings {
                conversation: room,
                operation: OperationId::from_local(sequence),
                command: ConversationCommand::SetEmojiAvatar {
                    emoji: text("🌿"),
                    color: Some(text("237B54")),
                },
            },
        )
        .await
        .unwrap();
    let R::Emoji { text: emoji, .. } = content(
        &mut alice,
        &runtime,
        a,
        &mut sequence,
        C::Asset(AssetRequest::ConversationAvatar(room)),
    )
    .await
    .unwrap() else {
        panic!("room emoji");
    };
    assert_eq!(emoji.as_str(), "🌿");
    println!("content-backend server-room-emoji: pass");
    let R::PollCapabilities(create) = content(
        &mut alice,
        &runtime,
        a,
        &mut sequence,
        poll(room, P::Capabilities),
    )
    .await
    .unwrap() else {
        panic!("capabilities");
    };
    assert_eq!(create.allowed, Decision::Allowed);
    let mut ids = Vec::new();
    for hidden in [true, false] {
        let R::Poll(created) = content(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            poll(room, P::Create(draft(hidden))),
        )
        .await
        .unwrap() else {
            panic!("created");
        };
        ids.push(created.id);
        let R::Poll(before) = content(
            &mut bob,
            &runtime,
            b,
            &mut sequence,
            poll(bob_room, P::Read(created.id)),
        )
        .await
        .unwrap() else {
            panic!("read");
        };
        assert!(!before.closed);
        assert_eq!(before.close.allowed, Decision::Denied);
        assert!(before.options.as_slice().iter().all(|o| o.votes.is_none()));
        let R::Poll(voted) = content(
            &mut bob,
            &runtime,
            b,
            &mut sequence,
            poll(
                bob_room,
                P::Vote {
                    poll: created.id,
                    options: page([0, 2]),
                },
            ),
        )
        .await
        .unwrap() else {
            panic!("vote");
        };
        assert!(voted.options.as_slice()[0].own && voted.options.as_slice()[2].own);
        if hidden {
            assert!(voted.options.as_slice().iter().all(|o| o.votes.is_none()));
            assert!(voted.details.as_slice().is_empty());
        } else {
            assert_eq!(voted.options.as_slice()[0].votes, Some(1));
        }
        let denied = content(
            &mut bob,
            &runtime,
            b,
            &mut sequence,
            poll(bob_room, P::Close(created.id)),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.kind, ErrorKind::PermissionDenied);
        let R::Poll(closed) = content(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            poll(room, P::Close(created.id)),
        )
        .await
        .unwrap() else {
            panic!("close");
        };
        assert!(closed.closed);
        assert_eq!(closed.voters, Some(1));
        assert_eq!(closed.options.as_slice()[0].votes, Some(1));
        assert_eq!(closed.options.as_slice()[2].votes, Some(1));
        if hidden {
            assert!(closed.details.as_slice().is_empty());
        } else {
            assert!(!closed.details.as_slice().is_empty());
        }
        let R::Poll(retained) = content(
            &mut bob,
            &runtime,
            b,
            &mut sequence,
            poll(bob_room, P::Read(created.id)),
        )
        .await
        .unwrap() else {
            panic!("retained");
        };
        assert!(retained.closed);
        assert_eq!(retained.options.as_slice()[0].votes, Some(1));
        let denied = content(
            &mut bob,
            &runtime,
            b,
            &mut sequence,
            poll(
                bob_room,
                P::Vote {
                    poll: created.id,
                    options: page([1]),
                },
            ),
        )
        .await
        .unwrap_err();
        assert_eq!(denied.kind, ErrorKind::Conflict);
        println!("content-backend poll-hidden-{hidden}-lifecycle-and-close-policy: pass");
    }
    // Only bounded, non-secret ids used by the independent raw API readback.
    std::fs::write(
        root.join("poll-readback.json"),
        format!("[{},{}]", ids[0], ids[1]),
    )
    .unwrap();

    for name in ["preview.png", "document.txt", "document.pdf", "cancel.bin"] {
        let local = root.join(name);
        let original = std::fs::read(&local).unwrap();
        let MR::Uploaded { path, bytes } = message(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            room,
            M::UploadFile {
                local_path: text(local.to_str().unwrap()),
                name: text(name),
            },
        )
        .await
        .unwrap() else {
            panic!("upload");
        };
        assert_eq!(bytes as usize, original.len());
        message(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            room,
            M::ShareFile {
                path,
                caption: text(""),
            },
        )
        .await
        .unwrap();
        let MR::Messages { messages, .. } = message(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            room,
            M::SharedFiles { before: None },
        )
        .await
        .unwrap() else {
            panic!("shared");
        };
        let shared = messages
            .as_slice()
            .iter()
            .find(|row| {
                row.rich
                    .as_slice()
                    .iter()
                    .any(|o| o.kind.as_str() == "file" && o.name.as_str() == name)
            })
            .expect("shared attachment");
        let object = shared
            .rich
            .as_slice()
            .iter()
            .find(|o| o.kind.as_str() == "file" && o.name.as_str() == name)
            .unwrap();
        assert!(object.id.is_some());
        assert!(object.mime_type.is_some());
        let attachment = |large| AssetRequest::Attachment {
            message: shared.message.id,
            placeholder: object.placeholder.clone(),
            large,
        };
        let R::CloudLink(link) = content(
            &mut alice,
            &runtime,
            a,
            &mut sequence,
            C::OpenCloud {
                message: shared.message.id,
                placeholder: object.placeholder.clone(),
            },
        )
        .await
        .unwrap() else {
            panic!("cloud link");
        };
        assert!(link.as_str().contains("/index.php/f/"));
        if name == "preview.png" {
            raster(
                content(
                    &mut alice,
                    &runtime,
                    a,
                    &mut sequence,
                    C::Asset(attachment(false)),
                )
                .await
                .unwrap(),
            );
            raster(
                content(
                    &mut alice,
                    &runtime,
                    a,
                    &mut sequence,
                    C::Asset(attachment(true)),
                )
                .await
                .unwrap(),
            );
            raster(
                content(
                    &mut alice,
                    &runtime,
                    a,
                    &mut sequence,
                    C::Asset(AssetRequest::MessageAvatar(shared.message.id)),
                )
                .await
                .unwrap(),
            );
            println!("content-backend image-small-large-and-message-avatar: pass");
        } else if name != "cancel.bin" {
            match content(
                &mut alice,
                &runtime,
                a,
                &mut sequence,
                C::Asset(attachment(true)),
            )
            .await
            {
                Ok(image) => {
                    raster(image);
                    println!("content-backend {name} server-raster-preview: available");
                }
                Err(error)
                    if matches!(error.kind, ErrorKind::NotFound | ErrorKind::Unsupported) =>
                {
                    println!(
                        "content-backend {name} server-raster-preview: unavailable; explicit-cloud-link-confirmed"
                    );
                }
                Err(error) => panic!("document preview {:?}", error.kind),
            }
        }
        let destination = root.join(format!("saved-{name}"));
        let progress = TransferProgress::default();
        let command = M::DownloadFile {
            message: shared.message.id,
            placeholder: object.placeholder.clone(),
            destination: text(destination.to_str().unwrap()),
            progress: progress.clone(),
        };
        if name == "cancel.bin" {
            let cancel = async {
                for _ in 0..10_000 {
                    if progress.snapshot().0 > 0 {
                        progress.cancel();
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                }
                panic!("download did not start");
            };
            let (result, ()) = tokio::join!(
                message(&mut alice, &runtime, a, &mut sequence, room, command),
                cancel
            );
            assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
            assert!(progress.snapshot().0 > 0);
            assert!(!progress.is_published());
            assert!(!destination.exists());
            assert!(
                !std::fs::read_dir(root).unwrap().any(|p| p
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".part"))
            );
            println!("content-backend cancelled-stream-private-staging-cleanup: pass");
        } else {
            let MR::FileSaved { bytes } =
                message(&mut alice, &runtime, a, &mut sequence, room, command)
                    .await
                    .unwrap()
            else {
                panic!("download");
            };
            assert_eq!(bytes as usize, original.len());
            assert_eq!(progress.snapshot().0, original.len() as u64);
            assert!(progress.is_published());
            assert_eq!(std::fs::read(&destination).unwrap(), original);
            let duplicate = M::DownloadFile {
                message: shared.message.id,
                placeholder: object.placeholder.clone(),
                destination: text(destination.to_str().unwrap()),
                progress: TransferProgress::default(),
            };
            assert_eq!(
                message(&mut alice, &runtime, a, &mut sequence, room, duplicate)
                    .await
                    .unwrap_err()
                    .kind,
                ErrorKind::Conflict
            );
            assert_eq!(std::fs::read(&destination).unwrap(), original);
            println!("content-backend {name} bytes-progress-atomic-no-overwrite: pass");
        }
    }
}
