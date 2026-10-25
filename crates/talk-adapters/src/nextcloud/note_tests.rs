use super::*;
use talk_core::protocol::messaging::{ComposeOptions, MessagingCommand, MessagingResult};

fn note() -> Value {
    let mut data = room();
    data["type"] = json!(6);
    data["objectType"] = json!("note_to_self");
    data["objectId"] = json!("fixture_user");
    data["participantType"] = json!(1);
    data["canLeaveConversation"] = json!(false);
    data
}
fn note_capabilities(major: u16) -> Value {
    let mut value = caps(major);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([json!("note-to-self"), json!("classified-conversations")]);
    value
}

#[test]
fn note_to_self_object_recognition_preserves_every_existing_policy_fence() {
    let id = ConversationId {
        account: scope().account,
        host: HostId::from_local(1),
        local: 7,
    };
    for major in [34, 35] {
        let caps = wire::Capabilities::decode(&note_capabilities(major)).unwrap();
        let allowed = wire::room(&note(), id, &caps).unwrap();
        assert_eq!(allowed.kind, talk_core::protocol::RoomKind::NoteToSelf);
        assert!(!allowed.unknown_restrictions);
        assert_eq!(allowed.actions.read_messages, Decision::Allowed);
        assert_eq!(allowed.actions.send_message, Decision::Allowed);
        assert_eq!(allowed.actions.join_call, Decision::Denied);
        for (field, value, read, send) in [
            ("readOnly", json!(1), Decision::Allowed, Decision::Denied),
            (
                "permissions",
                json!(383),
                Decision::Allowed,
                Decision::Denied,
            ),
            ("attributes", json!(4), Decision::Denied, Decision::Denied),
            (
                "attributes",
                json!(32),
                Decision::Unknown,
                Decision::Unknown,
            ),
            (
                "permissions",
                Value::Null,
                Decision::Unknown,
                Decision::Unknown,
            ),
            (
                "remoteServer",
                json!("https://remote.example.test"),
                Decision::Unknown,
                Decision::Unknown,
            ),
            (
                "objectType",
                json!("note_to_self_future"),
                Decision::Unknown,
                Decision::Unknown,
            ),
            ("type", json!(2), Decision::Unknown, Decision::Unknown),
        ] {
            let mut data = note();
            data[field] = value;
            let result = wire::room(&data, id, &caps).unwrap();
            assert_eq!(
                (result.actions.read_messages, result.actions.send_message),
                (read, send),
                "{major}: {field}"
            );
        }
        let mut data = note();
        data["lobbyState"] = json!(1);
        data["permissions"] = json!(503);
        let result = wire::room(&data, id, &caps).unwrap();
        assert_eq!(result.actions.read_messages, Decision::Denied);
        assert_eq!(result.actions.send_message, Decision::Denied);
        let missing_capability = wire::Capabilities::decode(&super::caps(major)).unwrap();
        let result = wire::room(&note(), id, &missing_capability).unwrap();
        assert_eq!(result.actions.read_messages, Decision::Unknown);
        assert_eq!(result.actions.send_message, Decision::Unknown);
    }
}

#[tokio::test]
async fn authorized_note_to_self_reaches_history_and_send_on_both_families() {
    for major in [34, 35] {
        let mut accepted = message();
        accepted["message"] = json!("Synthetic note");
        let mut sent = Reply::data(accepted);
        sent.echo_reference = true;
        let server = Server::start(vec![
            Reply::data(note_capabilities(major)),
            Reply::data(note()),
            Reply::data(json!([message()])),
            // Each operation refreshes capabilities before it reads room policy.
            Reply::data(note_capabilities(major)),
            Reply::data(note()),
            sent,
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = client.bind_room(&note()).unwrap();
        let history = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation,
                    operation: OperationId::from_local(1),
                    command: MessagingCommand::History {
                        before: None,
                        thread: None,
                    },
                },
            )
            .await
            .unwrap();
        assert!(matches!(history, ProtocolReply::Messaging {
            result: MessagingResult::Messages { ref messages, .. }, ..
        } if messages.as_slice().len() == 1));
        let sent = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation,
                    operation: OperationId::from_local(2),
                    command: MessagingCommand::Send {
                        text: Text::new("Synthetic note").unwrap(),
                        options: ComposeOptions::default(),
                    },
                },
            )
            .await
            .unwrap();
        assert!(matches!(
            sent,
            ProtocolReply::Messaging {
                result: MessagingResult::Message(ref message),
                ..
            } if message.own && message.message.own_operation.is_some()
                && message.message.text.as_str() == "Synthetic note"
        ));
        assert_eq!(server.count(), 6);
        let seen = server.seen.lock().unwrap();
        assert!(seen[2].starts_with(b"GET "));
        assert!(String::from_utf8_lossy(&seen[2]).contains("setReadMarker=0"));
        assert!(String::from_utf8_lossy(&seen[3]).contains("ocs/v2.php/cloud/capabilities"));
        assert!(seen[5].starts_with(b"POST "));
    }
}
