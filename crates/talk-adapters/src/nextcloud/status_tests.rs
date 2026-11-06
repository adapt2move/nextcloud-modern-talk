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

#[tokio::test]
async fn own_status_write_is_gated_reconciled_and_never_replayed() {
    use talk_core::preferences::{
        RequestId,
        account::{AccountPreferenceChange, StatusChoice},
    };
    for enabled in [false, true] {
        let mut capability = caps(35);
        capability["capabilities"]["user_status"] = json!({"enabled": enabled});
        let response = json!({"userId":"fixture_user", "status":"away"});
        let replies = if enabled {
            vec![
                Reply::data(capability),
                Reply::data(response.clone()),
                Reply::data(response),
            ]
        } else {
            vec![Reply::data(capability)]
        };
        let server = Server::start(replies).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let result = client
            .request(
                context(&runtime),
                ProtocolRequest::SetPreference {
                    request: RequestId(9),
                    change: AccountPreferenceChange::UserPresence(StatusChoice::Away),
                },
            )
            .await;
        if enabled {
            let ProtocolReply::Preferences(reply) = result.unwrap() else {
                panic!("status reply")
            };
            assert_eq!(reply.values.scope, scope());
            assert_eq!(reply.values.user_presence, Some(StatusChoice::Away));
            let requests = server.seen.lock().unwrap();
            assert_eq!(requests.len(), 3);
            let write = String::from_utf8_lossy(&requests[1]);
            assert!(
                write.starts_with(
                    "PUT /cloud/ocs/v2.php/apps/user_status/api/v1/user_status/status"
                )
            );
            assert!(write.contains("statusType=away"));
            assert!(
                String::from_utf8_lossy(&requests[2])
                    .starts_with("GET /cloud/ocs/v2.php/apps/user_status/api/v1/user_status")
            );
        } else {
            assert_eq!(result.unwrap_err().kind, ErrorKind::Unsupported);
            assert_eq!(server.count(), 1);
        }
    }
    let mut capability = caps(35);
    capability["capabilities"]["user_status"] = json!({"enabled": true});
    let mut lost_ack = Reply::data(json!({}));
    lost_ack.drop_write_ack = true;
    let server = Server::start(vec![Reply::data(capability), lost_ack]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let error = client
        .request(
            context(&runtime),
            ProtocolRequest::SetPreference {
                request: RequestId(10),
                change: AccountPreferenceChange::UserPresence(StatusChoice::DoNotDisturb),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Uncertain);
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn account_status_read_distinguishes_missing_from_temporary_failure() {
    use talk_core::preferences::RequestId;
    for (status, expected_observed) in [(404, true), (500, false)] {
        let mut capability = caps(35);
        capability["capabilities"]["user_status"] = json!({"enabled": true});
        let server = Server::start(vec![
            Reply::data(capability),
            Reply::envelope(status, "failure", status, json!([])),
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let ProtocolReply::Preferences(reply) = client
            .request(
                context(&runtime),
                ProtocolRequest::ReadPreferences {
                    request: RequestId(11),
                },
            )
            .await
            .unwrap()
        else {
            panic!("preferences reply")
        };
        assert_eq!(reply.values.user_status_enabled, Some(true));
        assert_eq!(reply.values.user_status_observed, expected_observed);
        assert_eq!(reply.values.user_presence, None);
    }
}
