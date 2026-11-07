use super::*;
use talk_core::protocol::messaging::{MessagingCommand as C, MessagingResult as R};

fn capabilities() -> Value {
    let mut value = caps(35);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([
            json!("chat-read-status"),
            json!("chat-unread"),
            json!("chat-read-last"),
        ]);
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
async fn first_unread_scan_requires_exhaustion_and_keeps_the_oldest_visible_page() {
    let mut newer = message();
    newer["id"] = json!(60);
    newer["expirationTimestamp"] = json!(0);
    let mut older = message();
    older["id"] = json!(20);
    older["expirationTimestamp"] = json!(0);
    let mut page_newer = Reply::data(json!([newer]));
    page_newer.headers = "X-Chat-Last-Given: 60\r\n".into();
    let mut page_older = Reply::data(json!([older]));
    page_older.headers = "X-Chat-Last-Given: 20\r\n".into();
    let mut end = Reply::data(json!([]));
    end.status = 304;
    end.body.clear();
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(room()),
        page_newer,
        page_older,
        end,
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let room = bind(&mut client);
    let R::Messages { messages, .. } =
        act(&mut client, &runtime, room, 1, C::UnreadAfter(None)).await
    else {
        panic!("unread page")
    };
    assert_eq!(messages.as_slice().len(), 1);
    assert_eq!(messages.as_slice()[0].message.id.sequence, 20);
    let requests = server.seen.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests.iter().all(|request| request.starts_with(b"GET ")));
    assert!(String::from_utf8_lossy(&requests[3]).contains("lastKnownMessageId=60"));
    assert!(String::from_utf8_lossy(&requests[4]).contains("lastKnownMessageId=20"));
}

#[tokio::test]
async fn unread_scan_skips_bounded_hidden_activity_without_marking_read() {
    let mut hidden = message();
    hidden["id"] = json!(71);
    hidden["systemMessage"] = json!("reaction");
    let mut visible = message();
    visible["id"] = json!(72);
    visible["expirationTimestamp"] = json!(0);
    let mut first = Reply::data(json!([hidden]));
    first.headers = "X-Chat-Last-Given: 71\r\n".into();
    let mut second = Reply::data(json!([visible]));
    second.headers = "X-Chat-Last-Given: 72\r\n".into();
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(room()),
        first,
        second,
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let room = bind(&mut client);
    let marker = MessageId {
        conversation: room,
        sequence: 60,
    };
    let R::Messages { messages, .. } =
        act(&mut client, &runtime, room, 1, C::UnreadAfter(Some(marker))).await
    else {
        panic!("unread page")
    };
    assert_eq!(messages.as_slice()[0].message.id.sequence, 72);
    let requests = server.seen.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests.iter().all(|request| request.starts_with(b"GET ")));
    assert!(String::from_utf8_lossy(&requests[3]).contains("lastKnownMessageId=71"));
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
async fn explicit_unread_from_rewinds_but_automatic_reads_stay_monotonic() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(read_room(90)),
        Reply::data(json!([])),
        Reply::data(capabilities()),
        Reply::data(read_room(20)),
    ])
    .await;
    let rt = runtime();
    let mut client = client(&server.origin, rt.clone());
    let room = bind(&mut client);
    let message = |sequence| MessageId {
        conversation: room,
        sequence,
    };
    assert!(matches!(
        act(&mut client, &rt, room, 1, C::MarkUnreadFrom(message(20))).await,
        R::Changed
    ));
    assert!(matches!(
        act(&mut client, &rt, room, 2, C::MarkRead(message(10))).await,
        R::Changed
    ));
    let requests = server.seen.lock().unwrap();
    let writes: Vec<_> = requests
        .iter()
        .filter(|request| request.starts_with(b"POST "))
        .collect();
    assert_eq!(writes.len(), 1);
    assert!(String::from_utf8_lossy(writes[0]).contains("lastReadMessage=20"));
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
