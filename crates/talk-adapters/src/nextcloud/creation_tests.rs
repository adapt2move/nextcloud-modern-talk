use super::*;
use talk_core::protocol::creation::Room;

fn capabilities() -> Value {
    let mut value = caps(35);
    let features = value["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap();
    features.extend([json!("conversation-creation-all"), json!("listable-rooms")]);
    value
}

#[tokio::test]
async fn creation_preserves_staged_people_merges_forced_values_and_reports_partial_invites() {
    let presets = json!([
        {"identifier":"default","name":"Default","description":"","parameters":{"roomType":2,"listable":0,"permissions":0}},
        {"identifier":"forced","name":"forced","description":"","parameters":{"permissions":1}},
        {"identifier":"presentation","name":"Presentation","description":"A preset","parameters":{"permissions":389}}
    ]);
    let mut created = room();
    created["invalidParticipants"] = json!({"users":["cara"]});
    let server = Server::start(vec![
        Reply::data(capabilities()),
        Reply::data(presets),
        Reply::data(capabilities()),
        Reply::data(json!([{"id":"bob","label":"Bob","source":"users"}])),
        Reply::data(capabilities()),
        Reply::data(capabilities()),
        Reply::data(json!([{"id":"cara","label":"Cara","source":"users"}])),
        Reply::data(capabilities()),
        Reply::data(capabilities()),
        Reply::envelope(202, "ok", 202, created),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let options = client
        .request(context(&runtime), ProtocolRequest::InspectCreation)
        .await
        .unwrap();
    assert!(matches!(options, ProtocolReply::CreationOptions(_)));
    let mut participants = Page::empty();
    for query in ["Bob", "Cara"] {
        let ProtocolReply::People(found) = client
            .request(
                context(&runtime),
                ProtocolRequest::FindPeople {
                    query: Text::new(query).unwrap(),
                },
            )
            .await
            .unwrap()
        else {
            panic!("directory")
        };
        let id = found.as_slice()[0].id;
        client
            .request(context(&runtime), ProtocolRequest::StageCreationPerson(id))
            .await
            .unwrap();
        participants.try_push(id).unwrap();
    }
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::CreateConversation(Room {
                name: Text::new("Fixture room").unwrap(),
                description: Text::new("Fixture description").unwrap(),
                public: false,
                preset: Some(Text::new("presentation").unwrap()),
                participants,
            }),
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        ProtocolReply::CreatedConversation {
            incomplete_invitations: true,
            ..
        }
    ));
    let calls = server.seen.lock().unwrap();
    let request = String::from_utf8_lossy(calls.last().unwrap());
    assert!(request.contains("participants%5Busers%5D%5B%5D=bob"));
    assert!(request.contains("participants%5Busers%5D%5B%5D=cara"));
    assert!(request.contains("permissions=1"));
    assert!(request.contains("preset=presentation"));
    assert!(request.contains("description=Fixture+description"));
}
