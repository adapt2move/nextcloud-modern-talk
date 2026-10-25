use super::*;
use talk_core::protocol::messaging::{MessagingCommand as M, MessagingResult as R};

fn fixture_message(token: &str) -> Value {
    json!({"id":91,"token":token,"actorType":"users","actorId":"fixture_user","actorDisplayName":"Fixture","messageType":"comment","message":"accepted exactly once","timestamp":1,"referenceId":"","isReplyable":true,"messageParameters":{},"reactions":{},"reactionsSelf":[]})
}

#[tokio::test]
async fn send_and_forward_recover_a_lost_write_ack_in_a_fresh_client_without_another_post() {
    for forward in [false, true] {
        let source_room = room();
        let destination_room = json!({"id":8,"token":"destination","type":2,"displayName":"Destination","participantType":3,"permissions":511,"readOnly":0,"lobbyState":0,"attributes":0,"canLeaveConversation":true,"isSensitive":false});
        let destination_token = if forward { "destination" } else { "testroom" };
        let mut dropped = Reply::data(json!([]));
        dropped.drop_write_ack = true;
        let mut history = Reply::data(json!([fixture_message(destination_token)]));
        history.history_reference = true;
        let mut replies = vec![Reply::data(caps(35)), Reply::data(source_room.clone())];
        if forward {
            replies.push(Reply::data(json!([fixture_message("testroom")])));
            replies.push(Reply::data(destination_room.clone()));
        }
        replies.push(dropped);
        replies.extend([
            Reply::data(caps(35)),
            Reply::data(if forward {
                destination_room.clone()
            } else {
                source_room.clone()
            }),
            history,
        ]);
        let server = Server::start(replies).await;
        let operation = OperationId::from_local(if forward { 701 } else { 700 });
        let first_runtime = runtime();
        let mut first = client(&server.origin, first_runtime.clone());
        let source = bind(&mut first);
        let destination = if forward {
            first.bind_room(&destination_room).unwrap()
        } else {
            source
        };
        let command = if forward {
            M::ForwardApproved {
                source: MessageId {
                    conversation: source,
                    sequence: 91,
                },
                destination,
                author: Text::new("Fixture").unwrap(),
                body: Text::new("accepted exactly once").unwrap(),
            }
        } else {
            M::Send {
                text: Text::new("accepted exactly once").unwrap(),
                options: Default::default(),
            }
        };
        let failed = first
            .request(
                context(&first_runtime),
                ProtocolRequest::Messaging {
                    conversation: source,
                    operation,
                    command,
                },
            )
            .await;
        assert_eq!(
            failed.err().unwrap().kind,
            ErrorKind::Uncertain,
            "forward={forward}, requests={}",
            server.count()
        );
        assert_eq!(first.sends.len(), 1);
        drop(first);

        // A new client has no PendingSend table. It reconstructs correlation
        // from the durable operation and reads the actual POST's reference.
        let next_runtime = runtime();
        let mut next = client(&server.origin, next_runtime.clone());
        let current = if forward {
            next.bind_room(&destination_room).unwrap()
        } else {
            bind(&mut next)
        };
        assert!(next.sends.is_empty());
        let recovered = next
            .request(
                context(&next_runtime),
                ProtocolRequest::Messaging {
                    conversation: current,
                    operation,
                    command: M::ReconcileSend,
                },
            )
            .await
            .unwrap();
        assert!(
            matches!(recovered,ProtocolReply::Messaging {result:R::Message(ref message),..}if message.own&&message.message.own_operation==Some(operation)&&message.message.id.sequence==91&&message.message.id.conversation==current)
        );
        let requests = server.seen.lock().unwrap();
        let posts: Vec<_> = requests
            .iter()
            .filter(|request| request.starts_with(b"POST "))
            .collect();
        assert_eq!(posts.len(), 1);
        assert!(String::from_utf8_lossy(posts[0]).starts_with(&format!(
            "POST /cloud/ocs/v2.php/apps/spreed/api/v1/chat/{destination_token}"
        )));
        let last = String::from_utf8_lossy(requests.last().unwrap());
        assert!(last.starts_with(&format!(
            "GET /cloud/ocs/v2.php/apps/spreed/api/v1/chat/{destination_token}?"
        )));
    }
}

#[tokio::test]
async fn a_fresh_client_cannot_recover_a_foreign_lookalike_message_or_issue_a_retry_post() {
    let history = Reply::data(json!([fixture_message("testroom")]));
    let server = Server::start(vec![Reply::data(caps(35)), Reply::data(room()), history]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let conversation = bind(&mut client);
    let operation = OperationId::from_local(900);
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation,
                operation,
                command: M::ReconcileSend,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        ProtocolReply::Messaging {
            result: R::Send(SendReceipt {
                state: SendState::Uncertain,
                ..
            }),
            ..
        }
    ));
    assert_eq!(server.count(), 3);
    assert!(
        server
            .seen
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.starts_with(b"GET "))
    );
}

#[tokio::test]
async fn a_changed_source_cannot_replace_the_forward_preview_during_durable_replay() {
    for changed_author in [false, true] {
        let mut fresh = fixture_message("testroom");
        if changed_author {
            fresh["actorDisplayName"] = json!("Changed author");
        } else {
            fresh["message"] = json!("Changed source content");
        }
        let server = Server::start(vec![
            Reply::data(caps(35)),
            Reply::data(room()),
            Reply::data(json!([fresh])),
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let source = bind(&mut client);
        let mut destination_room = room();
        destination_room["id"] = json!(8);
        destination_room["token"] = json!("destination");
        let destination = client.bind_room(&destination_room).unwrap();
        let result = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation: source,
                    operation: OperationId::from_local(1),
                    command: M::ForwardApproved {
                        source: MessageId {
                            conversation: source,
                            sequence: 91,
                        },
                        destination,
                        author: Text::new("Fixture").unwrap(),
                        body: Text::new("accepted exactly once").unwrap(),
                    },
                },
            )
            .await;
        assert_eq!(result.err().unwrap().kind, ErrorKind::Conflict);
        assert_eq!(server.count(), 3);
        assert!(
            server
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.starts_with(b"GET "))
        );
    }
}

#[tokio::test]
async fn a_confirmed_forward_reads_its_canonical_destination_row_before_the_pending_bubble_retires()
{
    let mut destination_room = room();
    destination_room["id"] = json!(8);
    destination_room["token"] = json!("destination");
    let mut accepted = Reply::data(fixture_message("destination"));
    accepted.echo_reference = true;
    let mut canonical = Reply::data(json!([fixture_message("destination")]));
    canonical.history_reference = true;
    let server = Server::start(vec![
        Reply::data(caps(35)),
        Reply::data(room()),
        Reply::data(json!([fixture_message("testroom")])),
        Reply::data(destination_room.clone()),
        accepted,
        Reply::data(caps(35)),
        Reply::data(destination_room.clone()),
        canonical,
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let source = bind(&mut client);
    let destination = client.bind_room(&destination_room).unwrap();
    let operation = OperationId::from_local(500);
    let forwarded = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: source,
                operation,
                command: M::ForwardApproved {
                    source: MessageId {
                        conversation: source,
                        sequence: 91,
                    },
                    destination,
                    author: Text::new("Fixture").unwrap(),
                    body: Text::new("accepted exactly once").unwrap(),
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        forwarded,
        ProtocolReply::Messaging {
            result: R::Send(SendReceipt {
                state: SendState::ServerAccepted(_),
                ..
            }),
            ..
        }
    ));
    let reconciled = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: destination,
                operation,
                command: M::ReconcileSend,
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(reconciled,ProtocolReply::Messaging {result:R::Message(ref row),..} if row.message.id.conversation==destination&&row.message.own_operation==Some(operation))
    );
    let requests = server.seen.lock().unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with(b"POST "))
            .count(),
        1
    );
    assert!(requests.iter().any(|request| {
        String::from_utf8_lossy(request)
            .starts_with("GET /cloud/ocs/v2.php/apps/spreed/api/v1/chat/destination/91/context")
    }));
}
