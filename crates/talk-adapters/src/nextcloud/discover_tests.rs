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
async fn discovery_uses_a_current_listing_ticket_and_retains_its_chat_session() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(json!([listed(false)])),
        Reply::data(capabilities()),
        Reply::data(room()),
        Reply::data({
            let mut r = room();
            r["sessionId"] = json!("synthetic-session");
            r
        }),
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
    let listing = String::from_utf8_lossy(&calls[1]);
    assert!(listing.contains("/v4/listed-room?"));
    assert!(listing.contains("searchTerm=fixture"));
    assert!(String::from_utf8_lossy(&calls[3]).starts_with("GET "));
    assert!(String::from_utf8_lossy(&calls[4]).starts_with("POST "));
    assert_eq!(calls.len(), 5);
}

#[tokio::test]
async fn discovery_reports_password_requirement_from_the_server() {
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(json!([listed(true)])),
        Reply::data(capabilities()),
        Reply::data(listed(true)),
        Reply::envelope(403, "failure", 403, json!({"error":"password"})),
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
    assert_eq!(error.kind, ErrorKind::PasswordRequired);
    assert_eq!(server.count(), 5);
}
