use super::*;
use talk_core::protocol::conversation::{ConversationCommand as C, Setting};
use talk_core::protocol::messaging::{
    MessagingCommand as M, MessagingFeature, MessagingResult as R,
};
fn features(names: &[&str]) -> Value {
    let mut c = caps(35);
    let f = c["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap();
    for name in names {
        f.push(json!(name))
    }
    c["capabilities"]["spreed"]["config"]["chat"]["read-privacy"] = json!(0);
    c
}
fn message(id: u64) -> Value {
    json!({"id":id,"token":"testroom","actorType":"users","actorId":"fixture_user","actorDisplayName":"Fixture","messageType":"comment","message":"original","timestamp":1,"referenceId":"","isReplyable":true,"messageParameters":{},"reactions":{},"reactionsSelf":[]})
}
#[tokio::test]
async fn reminder_writes_publish_only_confirmed_values_and_keep_read_failure_uncertain() {
    for major in [34, 35] {
        for case in ["confirmed", "wrong-target", "read-failed", "cancelled"] {
            let mut capabilities = caps(major);
            capabilities["capabilities"]["spreed"]["features"]
                .as_array_mut()
                .unwrap()
                .push(json!("remind-me-later"));
            let mut replies = vec![
                Reply::data(capabilities),
                Reply::data(room()),
                Reply::envelope(201, "ok", 201, json!([])),
            ];
            if case != "cancelled" {
                replies.push(if case == "read-failed" {
                    Reply::envelope(500, "failure", 500, json!([]))
                } else {
                    Reply::data(json!({"roomToken":"testroom", "messageId": if case == "wrong-target" {8} else {7}, "reminderTimestamp": 201}))
                });
            }
            let server = Server::start(replies).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            let conversation = bind(&mut client);
            let result = client
                .request(
                    context(&runtime),
                    ProtocolRequest::Messaging {
                        conversation,
                        operation: OperationId::from_local(1),
                        command: M::Reminder {
                            message: MessageId {
                                conversation,
                                sequence: 7,
                            },
                            timestamp: if case == "cancelled" { None } else { Some(200) },
                        },
                    },
                )
                .await;
            match case {
                "confirmed" => assert!(
                    matches!(result, Ok(ProtocolReply::Messaging { result: R::Reminder(Some(ref reminder)), .. }) if reminder.message.sequence == 7 && reminder.timestamp == 201)
                ),
                "cancelled" => assert!(matches!(
                    result,
                    Ok(ProtocolReply::Messaging {
                        result: R::Reminder(None),
                        ..
                    })
                )),
                _ => assert_eq!(result.err().unwrap().kind, ErrorKind::Uncertain),
            }
            assert_eq!(server.count(), if case == "cancelled" { 3 } else { 4 });
            let requests = server.seen.lock().unwrap();
            let mutation = String::from_utf8_lossy(&requests[2]);
            assert!(mutation.starts_with(if case == "cancelled" {
                "DELETE "
            } else {
                "POST "
            }));
            if case != "cancelled" {
                assert!(String::from_utf8_lossy(&requests[3]).starts_with(
                    "GET /cloud/ocs/v2.php/apps/spreed/api/v1/chat/testroom/7/reminder"
                ));
            }
        }
    }
}

#[tokio::test]
async fn reading_pins_never_grants_shared_pin_mutation_to_members() {
    for major in [34, 35] {
        for (role, expected) in [
            (1, Decision::Allowed),
            (2, Decision::Allowed),
            (3, Decision::Denied),
            (6, Decision::Allowed),
        ] {
            let mut capabilities = caps(major);
            capabilities["capabilities"]["spreed"]["features"]
                .as_array_mut()
                .unwrap()
                .push(json!("pinned-messages"));
            let mut data = room();
            data["participantType"] = json!(role);
            let server = Server::start(vec![Reply::data(capabilities), Reply::data(data)]).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            let conversation = bind(&mut client);
            let ProtocolReply::Messaging {
                result: R::Capabilities(value),
                ..
            } = client
                .request(
                    context(&runtime),
                    ProtocolRequest::Messaging {
                        conversation,
                        operation: OperationId::from_local(1),
                        command: M::Capabilities,
                    },
                )
                .await
                .unwrap()
            else {
                panic!("expected capabilities");
            };
            assert_eq!(value.pin_change, expected);
            assert!(
                value
                    .actions
                    .as_slice()
                    .iter()
                    .any(|(feature, action)| *feature == MessagingFeature::Pins
                        && action.allowed == Decision::Allowed)
            );
        }
    }
}
#[tokio::test]
async fn fresh_poll_notification_requires_known_policy_silent_and_current_user_status() {
    for (case, expected, observe_status) in [
        ("all", Decision::Allowed, true),
        ("dnd", Decision::Denied, true),
        ("unknown-status", Decision::Unknown, true),
        ("missing-status-endpoint", Decision::Unknown, true),
        ("wrong-user", Decision::Unknown, true),
        ("silent", Decision::Denied, false),
        ("missing-silent", Decision::Allowed, true),
        ("null-silent", Decision::Unknown, false),
        ("malformed-silent", Decision::Unknown, false),
        ("group-default", Decision::Unknown, false),
        ("direct-default", Decision::Allowed, true),
        ("own-mention", Decision::Allowed, true),
        ("display-mention", Decision::Unknown, false),
        ("thread", Decision::Unknown, false),
        ("self", Decision::Denied, false),
        ("edited", Decision::Denied, false),
        ("history", Decision::Denied, false),
    ] {
        let mut data = room();
        data["notificationLevel"] = json!(1);
        let mut incoming = message(8);
        incoming["actorId"] = json!("bob");
        incoming["systemMessage"] = json!("");
        incoming["silent"] = json!(false);
        let mut status = json!({"userId":"fixture_user","status":"online"});
        match case {
            "dnd" => status["status"] = json!("dnd"),
            "unknown-status" => status["status"] = json!("future-status"),
            "wrong-user" => status["userId"] = json!("another-user"),
            "silent" => incoming["silent"] = json!(true),
            "missing-silent" => {
                incoming.as_object_mut().unwrap().remove("silent");
            }
            "null-silent" => incoming["silent"] = Value::Null,
            "malformed-silent" => incoming["silent"] = json!("false"),
            "group-default" => data["notificationLevel"] = json!(0),
            "direct-default" => {
                data["notificationLevel"] = json!(0);
                data["type"] = json!(1);
            }
            "own-mention" => {
                data["notificationLevel"] = json!(2);
                incoming["message"] = json!("Hello {mention-user}");
                incoming["messageParameters"] =
                    json!({"mention-user":{"type":"user","id":"fixture_user","name":"Fixture"}});
            }
            "display-mention" => {
                data["notificationLevel"] = json!(2);
                incoming["message"] = json!("Hello @fixture_user");
            }
            "thread" => incoming["threadId"] = json!(7),
            "self" => incoming["actorId"] = json!("fixture_user"),
            "edited" => incoming["lastEditTimestamp"] = json!(2),
            _ => {}
        }
        let mut replies = vec![
            Reply::data(features(&["silent-send-state"])),
            Reply::data(data),
            Reply::data(json!([incoming])),
        ];
        if observe_status {
            replies.push(if case == "missing-status-endpoint" {
                Reply::envelope(404, "failure", 404, json!([]))
            } else {
                Reply::data(status)
            });
        }
        let server = Server::start(replies).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let command = if case == "history" {
            M::History {
                before: None,
                thread: None,
            }
        } else {
            M::Poll {
                after: MessageId {
                    conversation,
                    sequence: 7,
                },
                thread: None,
            }
        };
        let reply = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation,
                    operation: OperationId::from_local(1),
                    command,
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Messaging {
            result: R::Messages { messages, .. },
            ..
        } = reply
        else {
            panic!("expected bounded message page");
        };
        assert_eq!(messages.as_slice()[0].notify, expected, "{case}");
        assert_eq!(server.count(), if observe_status { 4 } else { 3 }, "{case}");
        if observe_status {
            let observed = server.seen.lock().unwrap();
            assert!(
                String::from_utf8_lossy(&observed[3])
                    .starts_with("GET /cloud/ocs/v2.php/apps/user_status/api/v1/user_status")
            );
        }
    }
}
#[tokio::test]
async fn reaction_permission_is_independent_of_chat_permission() {
    let mut room_data = room();
    room_data["permissions"] = json!(257);
    let server = Server::start(vec![
        Reply::data(features(&["react-permission", "chat-get-context"])),
        Reply::data(room_data),
        Reply::data(json!([message(7)])),
        Reply::data(json!({"👍":[
            {"actorType":"users","actorId":"fixture_user","actorDisplayName":"Fixture","timestamp":1},
            {"actorType":"users","actorId":"another","actorDisplayName":"Other","timestamp":2}
        ]})),
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let result = c
        .request(
            context(&r),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(1),
                command: M::React {
                    message: MessageId {
                        conversation: id,
                        sequence: 7,
                    },
                    emoji: Text::new("👍").unwrap(),
                    add: true,
                },
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        ProtocolReply::Messaging {
            result: R::Reactions { message, reactions },
            ..
        } if message.sequence == 7 && reactions.as_slice().len() == 1
            && reactions.as_slice()[0].count == 2 && reactions.as_slice()[0].own
    ));
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn message_actor_keys_distinguish_equal_names_and_actor_types() {
    let mut first = message(7);
    first["actorId"] = json!("one");
    let mut second = message(8);
    second["actorId"] = json!("two");
    let mut guest = message(9);
    guest["actorId"] = json!("one");
    guest["actorType"] = json!("guests");
    let mut unknown = message(10);
    unknown.as_object_mut().unwrap().remove("actorId");
    let server = Server::start(vec![
        Reply::data(features(&[])),
        Reply::data(room()),
        Reply::data(json!([first, second, guest, unknown])),
    ])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let conversation = bind(&mut client);
    let result = client
        .request(
            context(&runtime),
            ProtocolRequest::Messaging {
                conversation,
                operation: OperationId::from_local(1),
                command: M::History {
                    before: None,
                    thread: None,
                },
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Messaging {
        result: R::Messages { messages, .. },
        ..
    } = result
    else {
        panic!("expected history");
    };
    let rows = messages.as_slice();
    assert!(rows[0].author_key.is_some());
    assert_ne!(rows[0].author_key, rows[1].author_key);
    assert_ne!(rows[0].author_key, rows[2].author_key);
    assert_eq!(rows[3].author_key, None);
    assert!(
        rows.iter()
            .all(|row| row.message.author == rows[0].message.author)
    );
}
#[tokio::test]
async fn preserved_owner_cannot_delete_clear_or_change_guest_access() {
    for command in [
        C::Delete,
        C::ClearHistory,
        C::SetGuestAccess {
            enabled: true,
            password: Text::new("password").unwrap(),
        },
    ] {
        let mut room_data = room();
        room_data["attributes"] = json!(2);
        room_data["participantType"] = json!(1);
        room_data["canDeleteConversation"] = json!(true);
        let server = Server::start(vec![
            Reply::data(features(&["preserve-conversation", "clear-history"])),
            Reply::data(room_data),
        ])
        .await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let e = c
            .request(
                context(&r),
                ProtocolRequest::ConversationSettings {
                    conversation: id,
                    operation: OperationId::from_local(1),
                    command,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::PermissionDenied);
        assert_eq!(server.count(), 2);
    }
}
#[tokio::test]
async fn poll_coalesces_edit_delete_without_restoring_deleted_text() {
    let original = message(7);
    let mut edited = message(8);
    edited["messageType"] = json!("system");
    edited["systemMessage"] = json!("message_edited");
    edited["parent"] = original.clone();
    edited["parent"]["message"] = json!("edited");
    let mut deleted = message(9);
    deleted["messageType"] = json!("system");
    deleted["systemMessage"] = json!("message_deleted");
    deleted["parent"] = original.clone();
    let mut history = Reply::data(json!([deleted, original, edited]));
    history.headers = "X-Chat-Last-Given: 9\r\n".into();
    let server = Server::start(vec![
        Reply::data(features(&[])),
        Reply::data(room()),
        history,
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let reply = c
        .request(
            context(&r),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(1),
                command: M::Poll {
                    after: MessageId {
                        conversation: id,
                        sequence: 6,
                    },
                    thread: None,
                },
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Messaging {
        result: R::Messages {
            messages, cursor, ..
        },
        ..
    } = reply
    else {
        panic!("messages")
    };
    assert_eq!(messages.as_slice().len(), 1);
    assert_eq!(messages.as_slice()[0].message.kind, MessageKind::Deleted);
    assert_eq!(messages.as_slice()[0].message.text.as_str(), "");
    assert_eq!(cursor.unwrap().sequence, 9);
    let req = String::from_utf8_lossy(&server.seen.lock().unwrap()[2]).into_owned();
    assert!(req.contains("markNotificationsAsRead=0"));
}
#[tokio::test]
async fn channel_member_list_and_classified_exposure_remain_closed() {
    for (attrs, command, features_extra) in [
        (8, C::Members, vec!["announcement-preset"]),
        (
            4,
            C::SetBotEnabled {
                bot: talk_core::protocol::conversation::IntegrationId {
                    conversation: ConversationId {
                        account: scope().account,
                        host: HostId::from_local(1),
                        local: 999,
                    },
                    local: 1,
                },
                enabled: true,
            },
            vec!["classified-conversations", "bots-v1"],
        ),
    ] {
        let mut room_data = room();
        room_data["attributes"] = json!(attrs);
        if attrs == 4 {
            room_data["participantType"] = json!(1)
        }
        let server = Server::start(vec![
            Reply::data(features(&features_extra)),
            Reply::data(room_data),
        ])
        .await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        let id = bind(&mut c);
        let e = c
            .request(
                context(&r),
                ProtocolRequest::ConversationSettings {
                    conversation: id,
                    operation: OperationId::from_local(1),
                    command,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.kind, ErrorKind::PermissionDenied);
        assert_eq!(server.count(), 2);
    }
}
#[tokio::test]
async fn unsupported_provider_and_unknown_permissions_are_truthful() {
    let mut room_data = room();
    room_data["permissions"] = json!(1023);
    let server = Server::start(vec![
        Reply::data(features(&["edit-messages"])),
        Reply::data(room_data),
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let id = bind(&mut c);
    let reply = c
        .request(
            context(&r),
            ProtocolRequest::Messaging {
                conversation: id,
                operation: OperationId::from_local(1),
                command: M::Capabilities,
            },
        )
        .await
        .unwrap();
    let ProtocolReply::Messaging {
        result: R::Capabilities(caps),
        ..
    } = reply
    else {
        panic!("capabilities")
    };
    assert!(
        caps.actions
            .as_slice()
            .iter()
            .any(|(f, a)| *f == MessagingFeature::Translation && !a.implemented)
    );
    assert!(
        caps.actions
            .as_slice()
            .iter()
            .any(|(f, a)| *f == MessagingFeature::Send && a.allowed == Decision::Unknown)
    );
}
#[test]
fn settings_unknown_attributes_never_grant_owner_mutation() {
    let c = wire::Capabilities::decode(&features(&["room-description"])).unwrap();
    let mut d = room();
    d["attributes"] = json!(128);
    d["participantType"] = json!(1);
    let id = ConversationId {
        account: scope().account,
        host: HostId::from_local(1),
        local: 1,
    };
    let room = wire::room(&d, id, &c).unwrap();
    let a = conversation::control(Setting::Metadata, &room, &d, &c);
    assert_eq!(a.allowed, Decision::Unknown);
}

#[tokio::test]
async fn inherited_permissions_survive_typed_api_and_adapter_projection() {
    for (raw, expected) in [
        (json!(502), Some(502)),
        (json!(132), Some(132)),
        (json!(0), Some(0)),
        (json!(512), None),
        (json!(-1), None),
        (Value::Null, None),
    ] {
        let mut advertised = features(&["conversation-permissions"]);
        advertised["capabilities"]["spreed"]["config"]["permissions"]["default"] = raw;
        let mut room_data = room();
        room_data["defaultPermissions"] = json!(0);
        // Moderator effective rights must never substitute for inherited defaults.
        room_data["permissions"] = json!(510);
        let server = Server::start(vec![Reply::data(advertised), Reply::data(room_data)]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let reply = client
            .request(
                context(&runtime),
                ProtocolRequest::ConversationSettings {
                    conversation,
                    operation: OperationId::from_local(1),
                    command: C::Read,
                },
            )
            .await
            .unwrap();
        let ProtocolReply::ConversationSettings {
            result: talk_core::protocol::conversation::ConversationResult::Details(details),
            ..
        } = reply
        else {
            panic!("expected authoritative settings");
        };
        assert_eq!(details.default_permissions, Some(0));
        assert_eq!(details.server_default_permissions, expected);
        assert_eq!(server.count(), 2);
    }
}
