use super::*;

fn capabilities() -> Value {
    let mut value = caps(35);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("listable-rooms"));
    value
}
fn listed(password: bool) -> Value {
    let mut value = room();
    value["listable"] = json!(1);
    value["hasPassword"] = json!(password);
    value["description"] = json!("Synthetic description");
    value
}

#[tokio::test]
async fn discovery_uses_a_current_listing_ticket_and_closes_its_temporary_session() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(json!([listed(false)])),
        Reply::data(capabilities()),
        Reply::data(json!({"sessionId":"synthetic-session"})),
        Reply::data(json!([])),
        Reply::data(room()),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let ProtocolReply::OpenConversations(found) = client
        .request(
            context(&runtime),
            ProtocolRequest::ListOpenConversations {
                query: Text::new("fixture").unwrap(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("listing")
    };
    let entry = &found.rooms.as_slice()[0];
    assert_eq!(
        entry.description.as_ref().unwrap().as_str(),
        "Synthetic description"
    );
    assert_eq!(entry.participants, None);
    let id = entry.id;
    let ProtocolReply::Conversation(joined) = client
        .request(context(&runtime), ProtocolRequest::JoinOpenConversation(id))
        .await
        .unwrap()
    else {
        panic!("join")
    };
    assert_eq!(joined.id, id);
    let calls = server.seen.lock().unwrap();
    assert!(String::from_utf8_lossy(&calls[1]).contains("/v4/listed-room?searchTerm=fixture"));
    assert!(String::from_utf8_lossy(&calls[3]).starts_with("POST "));
    assert!(String::from_utf8_lossy(&calls[4]).starts_with("DELETE "));
}

#[tokio::test]
async fn discovery_does_not_join_password_protected_or_unlisted_rooms() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(json!([listed(true)])),
        Reply::data(capabilities()),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let ProtocolReply::OpenConversations(found) = client
        .request(
            context(&runtime),
            ProtocolRequest::ListOpenConversations {
                query: Text::new("").unwrap(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("listing")
    };
    let id = found.rooms.as_slice()[0].id;
    let error = client
        .request(context(&runtime), ProtocolRequest::JoinOpenConversation(id))
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert_eq!(server.count(), 3);
}
