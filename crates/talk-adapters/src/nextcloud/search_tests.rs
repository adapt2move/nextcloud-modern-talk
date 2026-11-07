use super::*;
use talk_core::protocol::search::Query;

fn capabilities() -> Value {
    let mut value = caps(35);
    value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .push(json!("unified-search"));
    value
}
fn providers() -> Value {
    json!([{"id":"talk-message","filters":{"term":"string","conversation":"string","person":"person","since":"datetime","until":"datetime"}}])
}
fn query(room: Option<ConversationId>) -> Query {
    Query {
        term: Text::new("synthetic").unwrap(),
        conversation: room,
        person: None,
        since: None,
        until: None,
        cursor: None,
    }
}
#[tokio::test]
async fn account_search_maps_only_owned_room_handles_and_preserves_cursor() {
    let server = Server::start(vec![Reply::data(capabilities()), Reply::data(providers()), Reply::data(json!({"entries":[
        {"title":"Alice","subline":"Synthetic match","attributes":{"conversation":"testroom","messageId":"7"}},
        {"title":"Bob","subline":"Other room","attributes":{"conversation":"unboundroom","messageId":"8"}}
    ],"cursor":50}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let room = bind(&mut client);
    let ProtocolReply::SearchMessages(results) = client
        .request(
            context(&runtime),
            ProtocolRequest::SearchMessages(query(None)),
        )
        .await
        .unwrap()
    else {
        panic!("search")
    };
    assert_eq!(results.hits.as_slice().len(), 1);
    assert_eq!(
        results.hits.as_slice()[0].message,
        MessageId {
            conversation: room,
            sequence: 7
        }
    );
    assert_eq!(results.cursor.unwrap().as_str(), "50");
    let calls = server.seen.lock().unwrap();
    let search = String::from_utf8_lossy(&calls[2]);
    assert!(search.contains("/search/providers/talk-message/search?"));
    assert!(!search.contains("conversation="));
}
#[tokio::test]
async fn account_search_rejects_wrong_room_and_missing_filter_before_query() {
    for wrong_scope in [false, true] {
        let mut discovery = providers();
        if !wrong_scope {
            discovery[0]["filters"]
                .as_object_mut()
                .unwrap()
                .remove("conversation");
        }
        let server = Server::start(vec![Reply::data(capabilities()), Reply::data(discovery)]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let mut room = bind(&mut client);
        if wrong_scope {
            room.account = AccountId::from_local(999);
        }
        let error = client
            .request(
                context(&runtime),
                ProtocolRequest::SearchMessages(query(Some(room))),
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            if wrong_scope {
                ErrorKind::WrongScope
            } else {
                ErrorKind::Unsupported
            }
        );
        assert_eq!(server.count(), 2);
    }
}
