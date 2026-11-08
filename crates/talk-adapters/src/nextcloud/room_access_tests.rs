use super::*;
use talk_core::policy::Decision;

fn public(role: u8) -> Value {
    let mut r = room();
    r["type"] = json!(3);
    r["participantType"] = json!(role);
    r["sessionId"] = json!(if role == 0 { "0" } else { "chat-session" });
    r
}

#[tokio::test]
async fn public_entry_joins_before_evaluating_the_returned_participant_policy() {
    let server = Server::start(vec![
        Reply::data(caps(35)),
        Reply::data(public(0)),
        Reply::data(public(5)),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let ProtocolReply::Conversation(room) = client
        .request(
            context(&runtime),
            ProtocolRequest::OpenConversation {
                conversation: id,
                password: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("room")
    };
    assert_eq!(room.actions.read_messages, Decision::Allowed);
    assert_eq!(room.actions.send_message, Decision::Allowed);
    assert!(room.call_scope.is_none());
    assert!(client.active.is_none());
    assert_eq!(client.chat_session, Some(id));
    let seen = server.seen.lock().unwrap();
    assert!(String::from_utf8_lossy(&seen[2]).starts_with(
        "POST /cloud/ocs/v2.php/apps/spreed/api/v4/room/testroom/participants/active"
    ));
    assert!(String::from_utf8_lossy(&seen[2]).contains("force=false"));
    assert_eq!(seen.len(), 3);
}

#[tokio::test]
async fn password_lobby_ban_and_unknown_policy_remain_distinct() {
    for (response, expected) in [
        (
            Reply::envelope(403, "failure", 403, json!({"error":"password"})),
            ErrorKind::PasswordRequired,
        ),
        (
            Reply::envelope(403, "failure", 403, json!({"error":"ban"})),
            ErrorKind::PermissionDenied,
        ),
        (
            Reply::envelope(404, "failure", 404, json!([])),
            ErrorKind::NotFound,
        ),
        (
            Reply::data({
                let mut r = public(5);
                r["lobbyState"] = json!(1);
                r["permissions"] = json!(129);
                r
            }),
            ErrorKind::Lobby,
        ),
        (
            Reply::data({
                let mut r = public(5);
                r["permissions"] = json!(1024);
                r
            }),
            ErrorKind::Unsupported,
        ),
    ] {
        let server = Server::start(vec![
            Reply::data(caps(35)),
            Reply::data(public(0)),
            response,
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let id = bind(&mut client);
        let error = client
            .request(
                context(&runtime),
                ProtocolRequest::OpenConversation {
                    conversation: id,
                    password: Some(Text::new("synthetic-password").unwrap()),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(client.active.is_none());
        assert!(client.observed_call.is_none());
    }
}

#[tokio::test]
async fn public_entry_preserves_returned_read_only_permissions() {
    let mut read_only = public(5);
    read_only["permissions"] = json!(127);
    let server = Server::start(vec![
        Reply::data(caps(34)),
        Reply::data(public(0)),
        Reply::data(read_only),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let ProtocolReply::Conversation(room) = client
        .request(
            context(&runtime),
            ProtocolRequest::OpenConversation {
                conversation: id,
                password: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("room")
    };
    assert_eq!(room.actions.read_messages, Decision::Allowed);
    assert_eq!(room.actions.send_message, Decision::Denied);
}

#[tokio::test]
async fn switching_chat_rooms_closes_only_the_owned_chat_session() {
    let first = public(5);
    let mut next_before = public(4);
    next_before["token"] = json!("nextroom");
    next_before["id"] = json!(8);
    let mut next_joined = next_before.clone();
    next_joined["participantType"] = json!(5);
    next_joined["sessionId"] = json!("next-chat-session");
    let server = Server::start(vec![
        Reply::data(caps(35)),
        Reply::data(public(4)),
        Reply::data(first),
        Reply::data(caps(35)),
        Reply::data(next_before.clone()),
        Reply::data(json!([])),
        Reply::data(next_joined),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let first_id = bind(&mut client);
    let next_id = client.bind_room(&next_before).unwrap();
    for id in [first_id, next_id] {
        client
            .request(
                context(&runtime),
                ProtocolRequest::OpenConversation {
                    conversation: id,
                    password: None,
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(client.chat_session, Some(next_id));
    assert!(client.active.is_none());
    let calls = server.seen.lock().unwrap();
    assert!(String::from_utf8_lossy(&calls[5]).starts_with(
        "DELETE /cloud/ocs/v2.php/apps/spreed/api/v4/room/testroom/participants/active"
    ));
    assert!(String::from_utf8_lossy(&calls[6]).starts_with(
        "POST /cloud/ocs/v2.php/apps/spreed/api/v4/room/nextroom/participants/active"
    ));
}

#[tokio::test]
async fn an_observed_lobby_blocks_history_with_a_lobby_result() {
    let mut lobby = public(5);
    lobby["lobbyState"] = json!(1);
    lobby["permissions"] = json!(129);
    let server = Server::start(vec![Reply::data(caps(35)), Reply::data(lobby)]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let error = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(1),
                command: talk_core::protocol::messaging::MessagingCommand::History {
                    before: None,
                    thread: None,
                },
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Lobby);
    assert_eq!(
        server.count(),
        2,
        "no history request while lobby authority denies it"
    );
}

#[tokio::test]
async fn a_mismatched_join_reply_retains_cleanup_without_authority() {
    let mut mismatched = public(5);
    mismatched["token"] = json!("otherroom");
    let server = Server::start(vec![
        Reply::data(caps(35)),
        Reply::data(public(4)),
        Reply::data(mismatched),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let error = client
        .request(
            context(&runtime),
            ProtocolRequest::OpenConversation {
                conversation: id,
                password: None,
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::WrongScope);
    assert_eq!(client.chat_session, Some(id));
    assert!(client.chat_join_uncertain);
    assert!(client.active.is_none());
}
