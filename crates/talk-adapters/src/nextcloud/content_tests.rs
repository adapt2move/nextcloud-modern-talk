use super::*;
use talk_core::protocol::content::{ContentCommand as C, ContentResult as R, PollCommand as P};
fn features() -> Value {
    let mut c = caps(35);
    c["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("talk-polls"));
    c
}
fn poll(author: &str, status: u8) -> Value {
    json!({"id":9,"question":"Which?","options":["A","B"],"votes":{},"votedSelf":[],"status":status,"resultMode":0,"maxVotes":1,"numVoters":0,"actorType":"users","actorId":author})
}
#[tokio::test]
async fn poll_author_closes_but_ordinary_voter_cannot_dispatch_close() {
    for (author, allowed) in [("fixture_user", true), ("another_user", false)] {
        let mut replies = vec![
            Reply::data(features()),
            Reply::data(room()),
            Reply::data(poll(author, 0)),
        ];
        if allowed {
            replies.push(Reply::data(poll(author, 1)));
        }
        let server = Server::start(replies).await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let result = c
            .request(
                context(&r),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(800),
                    command: C::Poll {
                        conversation: id,
                        command: P::Close(9),
                    },
                },
            )
            .await;
        if allowed {
            assert!(matches!(result,Ok(ProtocolReply::Content{result:R::Poll(p),..}) if p.closed));
            assert_eq!(server.count(), 4);
        } else {
            assert_eq!(result.err().unwrap().kind, ErrorKind::PermissionDenied);
            assert_eq!(server.count(), 3);
        }
    }
}
#[tokio::test]
async fn closed_poll_converges_without_replaying_delete() {
    let server = Server::start(vec![
        Reply::data(features()),
        Reply::data(room()),
        Reply::data(poll("another", 1)),
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let result = c
        .request(
            context(&r),
            ProtocolRequest::Content {
                operation: OperationId::from_local(800),
                command: C::Poll {
                    conversation: id,
                    command: P::Close(9),
                },
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(result,ProtocolReply::Content{result:R::Poll(p),..} if p.closed&&p.close.allowed==Decision::Denied)
    );
    assert_eq!(server.count(), 3);
}
#[tokio::test]
async fn unavailable_poll_capability_and_direct_create_cannot_dispatch_mutation() {
    for direct in [false, true] {
        let mut data = room();
        if direct {
            data["type"] = json!(1);
        }
        let server = Server::start(vec![
            Reply::data(if direct { features() } else { caps(35) }),
            Reply::data(data),
        ])
        .await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let result = c
            .request(
                context(&r),
                ProtocolRequest::Content {
                    operation: OperationId::from_local(800),
                    command: C::Poll {
                        conversation: id,
                        command: P::Capabilities,
                    },
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Content {
            result: R::PollCapabilities(a),
            ..
        } = result
        else {
            panic!("capabilities")
        };
        assert!(
            a.server != talk_core::protocol::FeatureSupport::Supported
                || a.allowed != Decision::Allowed
        );
        assert_eq!(server.count(), 2);
    }
}
#[tokio::test]
async fn classified_preview_cannot_fetch_message_or_binary() {
    let mut data = room();
    data["objectType"] = json!("classified");
    data["attributes"] = json!(4);
    let server = Server::start(vec![Reply::data(features()), Reply::data(data)]).await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let result = c
        .request(
            context(&r),
            ProtocolRequest::Content {
                operation: OperationId::from_local(800),
                command: C::Asset(talk_core::protocol::content::AssetRequest::Attachment {
                    message: MessageId {
                        conversation: id,
                        sequence: 1,
                    },
                    placeholder: Text::new("file").unwrap(),
                    large: false,
                }),
            },
        )
        .await;
    assert_eq!(result.err().unwrap().kind, ErrorKind::PermissionDenied);
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn shared_documents_and_images_merge_with_one_bounded_pagination_cursor() {
    let mut caps = features();
    caps["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!("rich-object-sharing"),
            json!("rich-object-list-media"),
        ]);
    let list = |range: std::ops::Range<u64>| {
        let mut rows = serde_json::Map::new();
        for id in range {
            let mut row = message();
            row["id"] = json!(id);
            rows.insert(id.to_string(), row);
        }
        Value::Object(rows)
    };
    let server = Server::start(vec![
        Reply::data(caps),
        Reply::data(room()),
        Reply::data(list(1..41)),
        Reply::data(list(41..81)),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(801),
                command: talk_core::protocol::messaging::MessagingCommand::SharedFiles {
                    before: None,
                },
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Messaging {
        result:
            talk_core::protocol::messaging::MessagingResult::Messages {
                messages, cursor, ..
            },
        ..
    } = result
    else {
        panic!("shared file page")
    };
    assert_eq!(messages.as_slice().len(), 50);
    assert_eq!(messages.as_slice().first().unwrap().message.id.sequence, 31);
    assert_eq!(messages.as_slice().last().unwrap().message.id.sequence, 80);
    assert_eq!(cursor.unwrap().sequence, 31);
    let seen = server.seen.lock().unwrap();
    assert!(String::from_utf8_lossy(&seen[2]).contains("objectType=file"));
    assert!(String::from_utf8_lossy(&seen[3]).contains("objectType=media"));
}
