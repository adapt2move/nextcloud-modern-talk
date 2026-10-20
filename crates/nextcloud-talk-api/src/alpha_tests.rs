use super::tests::*;
use super::*;
use serde_json::{Value, json};
use std::time::Duration;
fn message(id: u64, reference: &str) -> Value {
    json!({"id":id,"token":"testroom","actorType":"users","actorId":"synthetic-user","actorDisplayName":"Fixture","messageType":"comment","message":"hello","timestamp":1,"referenceId":reference,"isReplyable":true,"messageParameters":{},"reactions":{},"reactionsSelf":[]})
}
#[tokio::test]
async fn dropped_direct_creation_remains_uncertain_and_cannot_replay() {
    let mut response = reply(json!({"token":"directroom","type":1,"displayName":"Person"}));
    response.2 = Duration::from_millis(100);
    let server = Server::start(vec![response]).await;
    let (mut client, runtime) = client(&server);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(25),
            client.request(
                &context(&runtime),
                Request::CreateDirect {
                    user: "another-user".into()
                },
            )
        )
        .await
        .is_err()
    );
    let error = client
        .request(
            &context(&runtime),
            Request::CreateDirect {
                user: "another-user".into(),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::Uncertain);
    assert_eq!(server.count(), 1);
}
#[test]
fn future_history_cannot_mark_read_or_clear_notifications() {
    let p = plan(&Request::Chat {
        room: room(),
        command: chat::Command::Poll {
            after: 7,
            thread: Some(4),
        },
    })
    .unwrap();
    assert_eq!(p.0, Method::GET);
    for pair in [
        ("lookIntoFuture", "1"),
        ("timeout", "0"),
        ("setReadMarker", "0"),
        ("noStatusUpdate", "1"),
        ("markNotificationsAsRead", "0"),
        ("threadId", "4"),
    ] {
        assert!(p.2.iter().any(|(k, v)| *k == pair.0 && v == pair.1));
    }
}
#[test]
fn message_decode_preserves_rich_reactions_and_canonical_update() {
    let mut original = message(7, "");
    original["message"] = json!("<script>plain</script> {file}");
    original["reactions"] = json!({"👍":2});
    original["reactionsSelf"] = json!(["👍"]);
    original["messageParameters"] =
        json!({"file":{"type":"file","id":"1","name":"safe.txt","link":"javascript:alert(1)"}});
    let decoded = models::Message::decode(&original).unwrap();
    assert_eq!(decoded.reactions[0].count, 2);
    assert!(decoded.reactions[0].own);
    assert_eq!(decoded.rich[0].link.as_deref(), Some("javascript:alert(1)"));
    let mut event = message(9, "");
    event["messageType"] = json!("system");
    event["systemMessage"] = json!("message_deleted");
    event["parent"] = original;
    let decoded = models::Message::decode(&event).unwrap();
    let parent = decoded.update.unwrap();
    assert_eq!(parent.id, 7);
    assert_eq!(parent.message.as_deref(), Some(""));
    assert!(parent.rich.is_empty());
    assert!(parent.reactions.is_empty());
}
#[test]
fn malformed_or_cross_room_parent_cannot_smuggle_context() {
    let mut m = message(7, "");
    m["parent"] = message(2, "");
    m["parent"]["token"] = json!("otherroom");
    assert_eq!(
        models::Message::decode(&m).err().unwrap(),
        ErrorKind::WrongScope
    );
    m["parent"]["token"] = json!("testroom");
    m["parent"]["parent"] = message(1, "");
    assert_eq!(
        models::Message::decode(&m).err().unwrap(),
        ErrorKind::Malformed
    );
}
#[tokio::test]
async fn settled_send_journal_reclaims_but_uncertain_reference_survives() {
    let replies = (1..=125)
        .map(|n| reply(message(n, &format!("ref-{n}"))))
        .collect();
    let server = Server::start(replies).await;
    let (mut c, r) = client(&server);
    c.sends.push(SendAttempt {
        room: room(),
        reference: "uncertain-ref".into(),
        uncertain: true,
    });
    for n in 1..=125 {
        c.request(
            &context(&r),
            Request::SendMessage {
                room: room(),
                message: "hello".into(),
                reference: format!("ref-{n}"),
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(c.sends.len(), 100);
    let e = c
        .request(
            &context(&r),
            Request::SendMessage {
                room: room(),
                message: "duplicate".into(),
                reference: "uncertain-ref".into(),
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(e.kind, ErrorKind::Uncertain);
    assert_eq!(server.count(), 125);
}
#[tokio::test]
async fn schedule_drop_is_not_replayed_and_readback_is_room_scoped() {
    let mut delayed = reply(json!([]));
    delayed.2 = Duration::from_secs(1);
    let server = Server::start(vec![delayed]).await;
    let (mut c, r) = client(&server);
    let command = || Request::Chat {
        room: room(),
        command: chat::Command::Schedule {
            message: "future".into(),
            send_at: 9999999999,
            options: chat::ComposeOptions::default(),
        },
    };
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            c.request(&context(&r), command())
        )
        .await
        .is_err()
    );
    let e = c.request(&context(&r), command()).await.err().unwrap();
    assert_eq!(e.kind, ErrorKind::Uncertain);
    assert_eq!(server.count(), 1);
}
#[test]
fn routes_validate_and_place_delete_parameters_in_query() {
    let p = plan(&Request::Chat {
        room: room(),
        command: chat::Command::React {
            message: 9,
            reaction: "👍".into(),
            add: false,
        },
    })
    .unwrap();
    assert_eq!(p.0, Method::DELETE);
    assert!(p.2.iter().any(|(k, v)| *k == "reaction" && v == "👍"));
    assert!(p.3.is_empty());
    let p = plan(&Request::Conversation {
        room: room(),
        command: conversation::Command::RemoveAttendee(12),
    })
    .unwrap();
    assert_eq!(p.2, vec![("attendeeId", "12".into())]);
    assert!(
        plan(&Request::Conversation {
            room: room(),
            command: conversation::Command::DefaultPermissions(512)
        })
        .is_err()
    );
}

#[derive(Default)]
struct CountingSink {
    bytes: usize,
    largest: usize,
}
impl tokio::io::AsyncWrite for CountingSink {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.bytes += bytes.len();
        self.largest = self.largest.max(bytes.len());
        std::task::Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
#[tokio::test]
async fn binary_download_streams_beyond_json_ceiling_in_bounded_writes() {
    let server = Server::start(vec![(200, "a".repeat(1024 * 1024), Duration::ZERO)]).await;
    let (mut c, r) = client(&server);
    let mut sink = CountingSink::default();
    let bytes = c
        .download_file(
            &context(&r),
            files::FilePath::parse("/Talk/file.txt").unwrap(),
            &mut sink,
        )
        .await
        .unwrap();
    assert_eq!(bytes, 1024 * 1024);
    assert_eq!(sink.bytes, 1024 * 1024);
    assert!(sink.largest <= files::FILE_CHUNK_BYTES);
    assert_eq!(MAX_RESPONSE_BYTES, 256 * 1024);
}
#[tokio::test]
async fn binary_limits_and_create_only_conflict_do_not_weaken_json_or_replay() {
    let server = Server::start(vec![(412, String::new(), Duration::ZERO)]).await;
    let (mut c, r) = client(&server);
    assert_eq!(
        c.upload_file(
            &context(&r),
            files::FilePath::parse("/file").unwrap(),
            std::io::Cursor::new(Vec::<u8>::new()),
            files::MAX_FILE_BYTES + 1
        )
        .await
        .err()
        .unwrap()
        .kind,
        ErrorKind::TooLarge
    );
    assert_eq!(server.count(), 0);
    assert_eq!(
        c.upload_file(
            &context(&r),
            files::FilePath::parse("/file").unwrap(),
            std::io::Cursor::new(b"content".to_vec()),
            7
        )
        .await
        .err()
        .unwrap()
        .kind,
        ErrorKind::Conflict
    );
    assert_eq!(server.count(), 1);
    for path in [
        "/../secret",
        "/folder//file",
        "/folder/./file",
        "relative",
        "/folder\\file",
    ] {
        assert!(files::FilePath::parse(path).is_err())
    }
}

#[test]
fn emoji_avatar_uses_v1_and_validates_color_before_dispatch() {
    let p = plan(&Request::Conversation {
        room: room(),
        command: conversation::Command::EmojiAvatar {
            emoji: "🦀".into(),
            color: Some("#336699".into()),
        },
    })
    .unwrap();
    assert!(p.1.contains("/api/v1/room/"));
    assert!(p.3.contains(&("color", "336699".into())));
    assert!(
        plan(&Request::Conversation {
            room: room(),
            command: conversation::Command::EmojiAvatar {
                emoji: "🦀".into(),
                color: Some("red".into())
            }
        })
        .is_err()
    );
}

#[tokio::test]
async fn scheduled_edit_accepts_documented_202_envelope() {
    let data = json!({"id":"123456789012345678","message":"updated","sendAt":9999999999u64,"threadId":0,"silent":false});
    let server = Server::start(vec![(
        202,
        json!({"ocs":{"meta":{"status":"ok","statuscode":202},"data":data}}).to_string(),
        Duration::ZERO,
    )])
    .await;
    let (mut c, r) = client(&server);
    let reply = c
        .request(
            &context(&r),
            Request::Chat {
                room: room(),
                command: chat::Command::EditScheduled {
                    id: 123456789012345678,
                    message: "updated".into(),
                    send_at: 9999999999,
                    silent: false,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(reply,Reply::Chat(chat::Response::Schedule(s)) if s.id==123456789012345678));
}

#[tokio::test]
async fn published_poll_drop_retains_create_uncertainty_and_read_correlates_id() {
    let mut delayed = reply(json!([]));
    delayed.2 = Duration::from_secs(1);
    let server = Server::start(vec![delayed]).await;
    let (mut c, r) = client(&server);
    let command = || poll::Command::Create {
        question: "When?".into(),
        options: vec!["Today".into(), "Tomorrow".into()],
        hidden_results: true,
        max_votes: 1,
    };
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            c.poll(&context(&r), &room(), command())
        )
        .await
        .is_err()
    );
    assert_eq!(
        c.poll(&context(&r), &room(), command())
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Uncertain
    );
    assert_eq!(server.count(), 1);
    let server=Server::start(vec![reply(json!({"id":8,"question":"When?","options":["Today","Tomorrow"],"votes":[],"votedSelf":[],"status":0,"resultMode":0,"maxVotes":1,"numVoters":0,"actorType":"users","actorId":"synthetic-user"}))]).await;
    let (mut c, r) = client(&server);
    assert_eq!(
        c.poll(&context(&r), &room(), poll::Command::Read(7))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
}
#[tokio::test]
async fn streamed_download_reports_observed_bytes_and_content_length() {
    let server = Server::start(vec![(200, "a".repeat(1024 * 1024), Duration::ZERO)]).await;
    let (mut c, r) = client(&server);
    let observed = std::sync::Mutex::new(Vec::new());
    let progress = |received, total| observed.lock().unwrap().push((received, total));
    let mut sink = CountingSink::default();
    let bytes = c
        .download_file_progress(
            &context(&r),
            files::FilePath::parse("/progress.txt").unwrap(),
            &mut sink,
            &progress,
        )
        .await
        .unwrap();
    let values = observed.lock().unwrap();
    assert!(values.len() > 1);
    assert_eq!(values[0], (0, Some(bytes)));
    assert_eq!(values.last().copied(), Some((bytes, Some(bytes))));
    assert!(values.windows(2).all(|pair| pair[0].0 <= pair[1].0));
    assert_eq!(bytes, sink.bytes as u64);
}
#[test]
fn rich_numeric_poll_and_file_metadata_survive_typed_decode() {
    let mut m = message(8, "");
    m["messageParameters"] = json!({"poll":{"type":"talk-poll","id":42,"name":"Lunch"},"file":{"type":"file","id":3,"name":"notes.pdf","mimetype":"application/pdf","size":30,"path":"Talk/notes.pdf"}});
    let decoded = models::Message::decode(&m).unwrap();
    let poll = decoded.rich.iter().find(|r| r.kind == "talk-poll").unwrap();
    assert_eq!(poll.id.as_deref(), Some("42"));
    let file = decoded.rich.iter().find(|r| r.kind == "file").unwrap();
    assert_eq!(file.mime_type.as_deref(), Some("application/pdf"));
    assert_eq!(file.size, Some(30));
}

#[tokio::test]
async fn image_lane_rejects_non_image_body_and_ambiguous_avatar_targets() {
    let server = Server::start(vec![reply(json!({"not":"an image"}))]).await;
    let (mut client, runtime) = client(&server);
    let error = client
        .image(
            &context(&runtime),
            content::ImageRequest::UserAvatar {
                user: "synthetic-user".into(),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    for user in ["", ".", "..", "invalid\nuser"] {
        let error = client
            .image(
                &context(&runtime),
                content::ImageRequest::UserAvatar { user: user.into() },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::Malformed);
    }
    assert_eq!(server.count(), 1);
}
