use super::*;
use talk_core::protocol::messaging::{MessagingCommand as C, MessagingResult as R};

fn capabilities() -> Value {
    let mut value = caps(35);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([json!("chat-read-status"), json!("chat-unread")]);
    value["capabilities"]["spreed"]["config"]["chat"]["read-privacy"] = json!(0);
    value
}
fn read_room(read: u64) -> Value {
    let mut value = room();
    value["lastReadMessage"] = json!(read);
    value
}
async fn act(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    room: ConversationId,
    serial: u128,
    command: C,
) -> R {
    match client
        .request(
            context(runtime),
            ProtocolRequest::Messaging {
                conversation: room,
                operation: OperationId::from_local(serial),
                command,
            },
        )
        .await
        .unwrap()
    {
        ProtocolReply::Messaging { result, .. } => result,
        _ => panic!("messaging reply"),
    }
}

#[tokio::test]
async fn mark_unread_availability_tracks_the_server_capability() {
    use talk_core::protocol::messaging::MessagingFeature;
    for enabled in [false, true] {
        let server = Server::start(vec![
            Reply::data(if enabled { capabilities() } else { caps(35) }),
            Reply::data(room()),
        ])
        .await;
        let rt = runtime();
        let mut client = client(&server.origin, rt.clone());
        let room = bind(&mut client);
        let R::Capabilities(caps) = act(&mut client, &rt, room, 1, C::Capabilities).await else {
            panic!("capabilities")
        };
        let (_, availability) = caps
            .actions
            .as_slice()
            .iter()
            .find(|(feature, _)| *feature == MessagingFeature::MarkUnread)
            .unwrap();
        assert_eq!(
            availability.server == talk_core::protocol::FeatureSupport::Supported,
            enabled
        );
    }
}

#[tokio::test]
async fn old_visible_messages_cannot_rewind_server_or_runtime_read_progress() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(read_room(90)),
        Reply::data(capabilities()),
        Reply::data(read_room(0)),
        Reply::data(capabilities()),
        Reply::data(read_room(0)),
        Reply::data(json!([])),
        Reply::data(capabilities()),
        Reply::data(read_room(0)),
    ])
    .await;
    let rt = runtime();
    let mut client = client(&server.origin, rt.clone());
    let room = bind(&mut client);
    for (serial, sequence) in [(1, 80), (2, 20), (3, 100), (4, 70)] {
        assert!(matches!(
            act(
                &mut client,
                &rt,
                room,
                serial,
                C::MarkRead(MessageId {
                    conversation: room,
                    sequence
                })
            )
            .await,
            R::Changed
        ));
    }
    let seen = server.seen.lock().unwrap();
    let writes: Vec<_> = seen
        .iter()
        .filter(|request| request.starts_with(b"POST "))
        .collect();
    assert_eq!(writes.len(), 1);
    assert!(String::from_utf8_lossy(writes[0]).contains("lastReadMessage=100"));
}

#[tokio::test]
async fn explicit_mark_unread_releases_the_local_automatic_read_floor() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(read_room(90)),
        Reply::data(capabilities()),
        Reply::data(read_room(90)),
        Reply::data(json!([])),
        Reply::data(capabilities()),
        Reply::data(read_room(0)),
        Reply::data(json!([])),
    ])
    .await;
    let rt = runtime();
    let mut client = client(&server.origin, rt.clone());
    let room = bind(&mut client);
    act(
        &mut client,
        &rt,
        room,
        1,
        C::MarkRead(MessageId {
            conversation: room,
            sequence: 80,
        }),
    )
    .await;
    act(&mut client, &rt, room, 2, C::MarkUnread).await;
    act(
        &mut client,
        &rt,
        room,
        3,
        C::MarkRead(MessageId {
            conversation: room,
            sequence: 20,
        }),
    )
    .await;
    assert!(server.seen.lock().unwrap().iter().any(|request| {
        request.starts_with(b"POST ")
            && String::from_utf8_lossy(request).contains("lastReadMessage=20")
    }));
}

#[tokio::test]
async fn public_receipts_use_room_marker_when_history_header_is_missing_and_keep_explicit_zero() {
    for (room_marker, header, expected) in [(90, None, 90), (90, Some(0), 0)] {
        let mut data = room();
        data["lastCommonReadMessage"] = json!(room_marker);
        let mut history = Reply::data(json!([]));
        if let Some(read) = header {
            history.headers = format!("X-Chat-Last-Common-Read: {read}\r\n");
        }
        let server = Server::start(vec![
            Reply::data(capabilities()),
            Reply::data(data),
            history,
        ])
        .await;
        let rt = runtime();
        let mut client = client(&server.origin, rt.clone());
        let room = bind(&mut client);
        let result = act(
            &mut client,
            &rt,
            room,
            1,
            C::History {
                before: None,
                thread: None,
            },
        )
        .await;
        assert!(
            matches!(result, R::Messages { common_read: Some(read), .. } if read.sequence == expected)
        );
    }
}

#[tokio::test]
async fn private_or_unknown_receipt_policy_explicitly_retires_previous_receipts() {
    for privacy in [Value::Null, json!(1)] {
        let mut caps = capabilities();
        caps["capabilities"]["spreed"]["config"]["chat"]["read-privacy"] = privacy;
        let mut data = room();
        data["lastCommonReadMessage"] = json!(90);
        let mut history = Reply::data(json!([]));
        history.headers = "X-Chat-Last-Common-Read: 90\r\n".into();
        let server = Server::start(vec![Reply::data(caps), Reply::data(data), history]).await;
        let rt = runtime();
        let mut client = client(&server.origin, rt.clone());
        let room = bind(&mut client);
        let result = act(
            &mut client,
            &rt,
            room,
            1,
            C::History {
                before: None,
                thread: None,
            },
        )
        .await;
        assert!(
            matches!(result, R::Messages { common_read: Some(read), .. } if read.sequence == 0)
        );
    }
}
