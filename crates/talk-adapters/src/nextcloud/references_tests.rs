use super::*;
use talk_core::protocol::reference::ReferenceUrl;

#[tokio::test]
async fn resolved_card_never_fetches_an_external_thumbnail() {
    let url = "https://example.test/article";
    let server = Server::start(vec![Reply::data(caps(35)), Reply::data(room()), Reply::data(json!({"references": {
        (url): {"richObjectType":"open-graph","richObject":{"name":"A synthetic article","description":"A short description","thumb":"https://third-party.invalid/pixel"}}
    }}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let room = bind(&mut client);
    let ProtocolReply::Reference(Some(result)) = client
        .request(
            context(&runtime),
            ProtocolRequest::ResolveReference {
                message: MessageId {
                    conversation: room,
                    sequence: 9,
                },
                url: ReferenceUrl::parse(url).unwrap(),
            },
        )
        .await
        .unwrap()
    else {
        panic!("reference")
    };
    assert_eq!(result.title.as_str(), "A synthetic article");
    assert!(result.thumbnail.is_none());
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn an_unreadable_link_gets_no_card() {
    let url = "https://example.test/unreadable";
    // NextCloud's own answer for a page it could not read: the address as the name.
    let server = Server::start(vec![Reply::data(caps(35)), Reply::data(room()), Reply::data(json!({"references": {
        (url): {"richObjectType":"open-graph","richObject":{"id":url,"name":url,"description":null,"thumb":null,"link":url},
            "openGraphObject":{"id":url,"name":url,"description":null,"thumb":null,"link":url},"accessible":true}
    }}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let room = bind(&mut client);
    let reply = client
        .request(
            context(&runtime),
            ProtocolRequest::ResolveReference {
                message: MessageId {
                    conversation: room,
                    sequence: 9,
                },
                url: ReferenceUrl::parse(url).unwrap(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(reply, ProtocolReply::Reference(None)));
}
