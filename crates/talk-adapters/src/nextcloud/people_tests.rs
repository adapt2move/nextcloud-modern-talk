use super::*;
use talk_core::protocol::conversation::RecipientId;

fn creation_caps() -> Value {
    let mut value = caps(35);
    value["capabilities"]["spreed"]["config"]["conversations"]["can-create"] = json!(false);
    value
}
async fn find(client: &mut NextcloudClient, runtime: &NetworkRuntime) -> RecipientId {
    let reply = client
        .request(
            context(runtime),
            ProtocolRequest::FindPeople {
                query: Text::new("Person").unwrap(),
            },
        )
        .await
        .unwrap();
    let ProtocolReply::People(people) = reply else {
        panic!("people reply");
    };
    assert_eq!(people.as_slice().len(), 1);
    people.as_slice()[0].id
}

#[tokio::test]
async fn direct_creation_uses_current_account_directory_and_never_replays_an_operation() {
    let mut direct = room();
    direct["id"] = json!(9);
    direct["token"] = json!("directroom");
    direct["type"] = json!(1);
    let server = Server::start(vec![
        Reply::data(creation_caps()),
        Reply::data(json!([
            {"id":"fixture_user","label":"Me","source":"users"},
            {"id":"bob","label":"Bob","source":"users"},
            {"id":"team","label":"Team","source":"groups"}
        ])),
        Reply::data(creation_caps()),
        Reply::data(json!([{ "id":"cara", "label":"Cara", "source":"users" }])),
        Reply::data(creation_caps()),
        Reply::data(creation_caps()),
        Reply::data(creation_caps()),
        Reply::envelope(201, "ok", 201, direct.clone()),
        Reply::data(direct.clone()),
        Reply::data(creation_caps()),
        Reply::data(direct),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let stale = find(&mut client, &runtime).await;
    let person = find(&mut client, &runtime).await;
    assert_ne!(stale, person);
    let mut wrong = person;
    wrong.scope.account = AccountId::from_local(99);
    for recipient in [stale, wrong] {
        let error = client
            .request(
                context(&runtime),
                ProtocolRequest::CreateDirect {
                    operation: OperationId::from_local(1),
                    recipient,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::WrongScope);
    }
    let mut selected = None;
    for _ in 0..2 {
        let reply = client
            .request(
                context(&runtime),
                ProtocolRequest::CreateDirect {
                    operation: OperationId::from_local(2),
                    recipient: person,
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Conversation(room) = reply else {
            panic!("direct room");
        };
        assert_eq!(room.kind, talk_core::protocol::RoomKind::Direct);
        if let Some(id) = selected {
            assert_eq!(id, room.id);
        }
        selected = Some(room.id);
    }
    assert_eq!(server.count(), 11);
    let requests = server.seen.lock().unwrap();
    let posts: Vec<_> = requests
        .iter()
        .filter(|r| r.starts_with(b"POST "))
        .collect();
    assert_eq!(posts.len(), 1);
    let post = String::from_utf8_lossy(posts[0]);
    assert!(post.contains("roomType=1"));
    assert!(post.contains("invite=cara"));
    let search = String::from_utf8_lossy(&requests[1]);
    assert!(search.contains("itemId=new"));
    assert!(search.contains("shareTypes%5B%5D=0"));
}
