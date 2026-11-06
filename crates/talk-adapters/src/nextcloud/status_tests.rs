use super::*;
use talk_core::protocol::UserPresence;

#[tokio::test]
async fn room_status_is_requested_only_when_enabled_and_only_used_for_direct_rooms() {
    for (major, enabled) in [(34, Some(true)), (35, Some(false)), (35, None)] {
        let mut capabilities = caps(major);
        if let Some(enabled) = enabled {
            capabilities["capabilities"]["user_status"] = json!({"enabled": enabled});
        }
        let mut direct = room();
        direct["type"] = json!(1);
        direct["status"] = json!("away");
        direct["statusIcon"] = json!("☕");
        direct["statusMessage"] = json!("Break");
        let mut group = room();
        group["token"] = json!("grouproom");
        group["id"] = json!(8);
        group["status"] = json!("online");
        let server = Server::start(vec![
            Reply::data(capabilities),
            Reply::data(json!([direct, group])),
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let ProtocolReply::Conversations(list) = client
            .request(context(&runtime), ProtocolRequest::ListConversations)
            .await
            .unwrap()
        else {
            panic!("room list reply");
        };
        let rooms = list.rooms.as_slice();
        assert_eq!(rooms.len(), 2);
        if enabled == Some(true) {
            let status = rooms[0].user_status.as_ref().unwrap();
            assert_eq!(status.presence, UserPresence::Away);
            assert_eq!(status.icon.as_ref().unwrap().as_str(), "☕");
            assert_eq!(status.message.as_ref().unwrap().as_str(), "Break");
        } else {
            assert!(rooms[0].user_status.is_none());
        }
        assert!(rooms[1].user_status.is_none());
        let requests = server.seen.lock().unwrap();
        let request = String::from_utf8_lossy(&requests[1]);
        assert_eq!(
            request.contains("includeStatus=true"),
            enabled == Some(true)
        );
        assert!(request.contains("noStatusUpdate=1"));
    }
}

#[test]
fn unknown_and_missing_status_do_not_imply_online() {
    let mut data = caps(35);
    data["capabilities"]["user_status"] = json!({"enabled": true});
    let capabilities = wire::Capabilities::decode(&data).unwrap();
    let mut data = room();
    data["type"] = json!(1);
    let runtime = runtime();
    let mut client = client("http://127.0.0.1:1/cloud/", runtime);
    let id = client.bind_room(&data).unwrap();
    assert!(
        wire::room(&data, id, &capabilities)
            .unwrap()
            .user_status
            .is_none()
    );
    data["status"] = json!("future-status");
    assert_eq!(
        wire::room(&data, id, &capabilities)
            .unwrap()
            .user_status
            .unwrap()
            .presence,
        UserPresence::Unknown
    );
}
