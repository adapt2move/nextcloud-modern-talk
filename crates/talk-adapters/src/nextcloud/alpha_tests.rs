use super::*;
use talk_core::protocol::conversation::{ConversationCommand as C, Setting};
use talk_core::protocol::messaging::{
    MessagingCommand as M, MessagingFeature, MessagingResult as R,
};
use talk_core::protocol::{FeatureSupport, Role};
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
    let mut advertised = features(&["edit-messages", "translation", "chat-summary"]);
    advertised["capabilities"]["spreed"]["config"]["chat"]["has-translation-providers"] =
        json!(true);
    let server = Server::start(vec![Reply::data(advertised), Reply::data(room_data)]).await;
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
            .any(|(f, a)| *f == MessagingFeature::Translation
                && a.implemented
                && a.allowed == Decision::Unknown)
    );
    assert!(
        caps.actions
            .as_slice()
            .iter()
            .any(|(f, a)| *f == MessagingFeature::Summary
                && a.implemented
                && a.server == FeatureSupport::Unsupported
                && a.allowed == Decision::Unknown)
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

#[test]
fn csv_import_control_requires_its_feature_and_invitation_authority() {
    let mut data = room();
    data["participantType"] = json!(1);
    let id = ConversationId {
        account: scope().account,
        host: HostId::from_local(1),
        local: 1,
    };
    let caps = wire::Capabilities::decode(&features(&[])).unwrap();
    let ordinary = wire::room(&data, id, &caps).unwrap();
    let without_csv = conversation::control(Setting::CsvImport, &ordinary, &data, &caps);
    assert_eq!(without_csv.server, FeatureSupport::Unsupported);
    let caps = wire::Capabilities::decode(&features(&["email-csv-import"])).unwrap();
    let mut ordinary = wire::room(&data, id, &caps).unwrap();
    let available = conversation::control(Setting::CsvImport, &ordinary, &data, &caps);
    assert_eq!(available.server, FeatureSupport::Supported);
    assert_eq!(available.allowed, Decision::Allowed);
    ordinary.role = Role::Member;
    assert_eq!(
        conversation::control(Setting::CsvImport, &ordinary, &data, &caps).allowed,
        Decision::Denied
    );
    ordinary.role = Role::Owner;
    ordinary.classified = true;
    assert_eq!(
        conversation::control(Setting::CsvImport, &ordinary, &data, &caps).allowed,
        Decision::Denied
    );
    ordinary.classified = false;
    ordinary.unknown_restrictions = true;
    assert_eq!(
        conversation::control(Setting::CsvImport, &ordinary, &data, &caps).allowed,
        Decision::Unknown
    );
}

#[tokio::test]
async fn csv_preview_reads_current_policy_and_never_submits_invitations() {
    for major in [34, 35] {
        let mut capabilities = caps(major);
        capabilities["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!("email-csv-import"));
        let mut data = room();
        data["participantType"] = json!(1);
        let server = Server::start(vec![Reply::data(capabilities), Reply::data(data)]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let reply = client.request(context(&runtime), ProtocolRequest::ConversationSettings {
            conversation, operation: OperationId::from_local(81),
            command: C::PreviewEmailCsv(Text::new("Name,Email\r\n\"Person, One\",\"one@example.test\"\r\nOther,ONE@EXAMPLE.TEST\r\nBad,invalid").unwrap()),
        }).await.unwrap();
        let ProtocolReply::ConversationSettings {
            result:
                talk_core::protocol::conversation::ConversationResult::EmailCsvPreview {
                    recipients,
                    duplicate_count,
                    invalid_count,
                },
            ..
        } = reply
        else {
            panic!("expected bounded CSV preview");
        };
        assert_eq!(recipients.as_slice().len(), 1);
        assert_eq!(
            recipients.as_slice().first().unwrap().label.as_str(),
            "one@example.test"
        );
        assert_eq!((duplicate_count, invalid_count), (1, 1));
        assert_eq!(server.count(), 2);
        assert!(
            server
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.starts_with(b"GET "))
        );
    }
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

#[tokio::test]
async fn scheduled_list_preserves_failed_and_rescheduled_deadlines() {
    for major in [34, 35] {
        let mut capabilities = caps(major);
        capabilities["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!("scheduled-messages"));
        let server = Server::start(vec![
            Reply::data(capabilities),
            Reply::data(room()),
            Reply::data(json!([
                {"id":"10", "message":"failed", "sendAt":0, "originalSendAt":200, "threadId":0, "silent":false},
                {"id":"11", "message":"rescheduled", "sendAt":400, "originalSendAt":200, "threadId":0, "silent":false},
                {"id":"12", "message":"pending", "sendAt":500, "threadId":0, "silent":false}
            ])),
        ]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let result = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation,
                    operation: OperationId::from_local(1),
                    command: M::Scheduled,
                },
            )
            .await
            .unwrap();
        let ProtocolReply::Messaging {
            result: R::Scheduled(items),
            ..
        } = result
        else {
            panic!("expected schedules");
        };
        let items = items.as_slice();
        assert_eq!(
            (items[0].send_at, items[0].original_send_at),
            (0, Some(200))
        );
        assert_eq!(
            (items[1].send_at, items[1].original_send_at),
            (400, Some(200))
        );
        assert_eq!((items[2].send_at, items[2].original_send_at), (500, None));
    }
}

#[tokio::test]
async fn upcoming_reminders_have_an_independent_server_capability() {
    for (names, upcoming) in [
        (vec!["remind-me-later"], FeatureSupport::Unsupported),
        (
            vec!["remind-me-later", "upcoming-reminders"],
            FeatureSupport::Supported,
        ),
    ] {
        let server = Server::start(vec![Reply::data(features(&names)), Reply::data(room())]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let reply = client
            .request(
                context(&runtime),
                ProtocolRequest::Messaging {
                    conversation,
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
            panic!("capabilities");
        };
        let get = |feature| {
            caps.actions
                .as_slice()
                .iter()
                .find(|(f, _)| *f == feature)
                .unwrap()
                .1
                .server
        };
        assert_eq!(get(MessagingFeature::Reminders), FeatureSupport::Supported);
        assert_eq!(get(MessagingFeature::UpcomingReminders), upcoming);
    }
}

#[tokio::test]
async fn account_reminders_do_not_require_a_selected_room_and_keep_unknown_rooms_partial() {
    for major in [34, 35] {
        for supported in [false, true] {
            let mut capabilities = caps(major);
            if supported {
                capabilities["capabilities"]["spreed"]["features"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("upcoming-reminders"));
            }
            let mut replies = vec![Reply::data(capabilities)];
            if supported {
                replies.push(Reply::data(json!([
                    {"roomToken":"testroom", "messageId":7, "reminderTimestamp":200},
                    {"roomToken":"otherroom", "messageId":8, "reminderTimestamp":300},
                    {"roomToken":"unknownroom", "messageId":9, "reminderTimestamp":400}
                ])));
            }
            let server = Server::start(replies).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            let first = bind(&mut client);
            let mut other = room();
            other["token"] = json!("otherroom");
            other["id"] = json!(2);
            let second = client.bind_room(&other).unwrap();
            let ProtocolReply::UpcomingReminders(result) = client
                .request(context(&runtime), ProtocolRequest::UpcomingReminders)
                .await
                .unwrap()
            else {
                panic!("expected reminders")
            };
            assert_eq!(result.supported, supported);
            assert_eq!(result.complete, !supported);
            if supported {
                assert_eq!(result.reminders.as_slice().len(), 2);
                assert_eq!(result.reminders.as_slice()[0].message.conversation, first);
                assert_eq!(result.reminders.as_slice()[1].message.conversation, second);
                let requests = server.seen.lock().unwrap();
                assert!(String::from_utf8_lossy(&requests[1]).starts_with(
                    "GET /cloud/ocs/v2.php/apps/spreed/api/v1/chat/upcoming-reminders"
                ));
            } else {
                assert!(result.reminders.as_slice().is_empty());
                assert_eq!(server.count(), 1);
            }
        }
    }
}

#[tokio::test]
async fn translation_requires_provider_detection_and_matching_source() {
    for major in [34, 35] {
        for case in ["disabled", "no-detection", "changed", "translated"] {
            let mut capabilities = caps(major);
            capabilities["capabilities"]["spreed"]["config"]["chat"]["has-translation-providers"] =
                json!(case != "disabled");
            let mut replies = vec![Reply::data(capabilities), Reply::data(room())];
            if case != "disabled" {
                replies.push(Reply::data(json!([message(7)])));
                if case != "changed" {
                    replies.push(Reply::data(json!({"languageDetection": case != "no-detection", "languages":[{"from":"en", "to":"de"}]})));
                    if case == "translated" {
                        replies.push(Reply::data(json!({"text":"übersetzt", "from":"en"})));
                    }
                }
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
                        command: M::Translate {
                            message: MessageId {
                                conversation,
                                sequence: 7,
                            },
                            language: Text::new("de").unwrap(),
                            source: Text::new(if case == "changed" {
                                "outdated"
                            } else {
                                "original"
                            })
                            .unwrap(),
                        },
                    },
                )
                .await;
            match case {
                "disabled" | "no-detection" => {
                    assert_eq!(result.err().unwrap().kind, ErrorKind::Unsupported)
                }
                "changed" => assert_eq!(result.err().unwrap().kind, ErrorKind::Conflict),
                _ => assert!(
                    matches!(result, Ok(ProtocolReply::Messaging { result: R::Translation { text, .. }, .. }) if text.as_str() == "übersetzt")
                ),
            }
            assert_eq!(
                server.count(),
                match case {
                    "disabled" => 2,
                    "changed" => 3,
                    "no-detection" => 4,
                    _ => 5,
                }
            );
        }
    }
}

#[tokio::test]
async fn summary_requires_capability_threshold_and_correlated_task() {
    for major in [34, 35] {
        for case in ["not-advertised", "below-threshold", "ready", "wrong-owner"] {
            let mut capabilities = caps(major);
            if case != "not-advertised" {
                capabilities["capabilities"]["spreed"]["features"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!("chat-summary-api"));
            }
            capabilities["capabilities"]["spreed"]["config"]["chat"]["summary-threshold"] =
                json!(100);
            let mut data = room();
            data["lastReadMessage"] = json!(7);
            data["unreadMessages"] = json!(if case == "below-threshold" { 99 } else { 120 });
            let mut replies = vec![Reply::data(capabilities.clone()), Reply::data(data.clone())];
            if matches!(case, "ready" | "wrong-owner") {
                replies.extend([
                    Reply::data(capabilities.clone()), Reply::data(data.clone()),
                    Reply::envelope(201, "ok", 201, json!({"taskId":44, "nextOffset":17})),
                    Reply::data(capabilities.clone()), Reply::data(data.clone()),
                    Reply::data(json!({"task":{"id":44,"type":"core:text2text:summary","status":"STATUS_SUCCESSFUL", "appId":"spreed", "userId":if case == "wrong-owner" {"someone_else"} else {"fixture_user"}, "customId":"summary/testroom", "output":{"output":"A bounded summary."}}})),
                ]);
            }
            let server = Server::start(replies).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            let conversation = bind(&mut client);
            let ProtocolReply::Messaging {
                result: R::Capabilities(projected),
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
                panic!("capabilities")
            };
            assert_eq!(
                projected.summary_offer.is_some(),
                matches!(case, "ready" | "wrong-owner")
            );
            if matches!(case, "ready" | "wrong-owner") {
                let handle = OperationId::from_local(2);
                let start = client
                    .request(
                        context(&runtime),
                        ProtocolRequest::Messaging {
                            conversation,
                            operation: handle,
                            command: M::Summarize {
                                from: MessageId {
                                    conversation,
                                    sequence: 7,
                                },
                            },
                        },
                    )
                    .await
                    .unwrap();
                assert!(
                    matches!(start, ProtocolReply::Messaging { result: R::SummaryStarted { handle: got }, .. } if got == handle)
                );
                let status = client
                    .request(
                        context(&runtime),
                        ProtocolRequest::Messaging {
                            conversation,
                            operation: OperationId::from_local(3),
                            command: M::SummaryStatus { handle },
                        },
                    )
                    .await;
                if case == "wrong-owner" {
                    assert_eq!(status.err().unwrap().kind, ErrorKind::WrongScope);
                } else {
                    assert!(
                        matches!(status, Ok(ProtocolReply::Messaging { result: R::SummaryPart { text, more: true }, .. }) if text.as_str() == "A bounded summary.")
                    );
                }
                let observed = server.seen.lock().unwrap();
                assert!(String::from_utf8_lossy(&observed[4]).starts_with(
                    "POST /cloud/ocs/v2.php/apps/spreed/api/v1/chat/testroom/summarize"
                ));
                assert!(
                    String::from_utf8_lossy(&observed[7])
                        .starts_with("GET /cloud/ocs/v2.php/taskprocessing/task/44")
                );
            }
        }
    }
}

#[tokio::test]
async fn federation_invitation_actions_are_capability_and_account_scoped() {
    for major in [34, 35] {
        for case in ["disabled", "wrong-user", "accept", "decline"] {
            let mut capabilities = caps(major);
            capabilities["capabilities"]["spreed"]["features"].as_array_mut().unwrap().push(json!("federation-v1"));
            capabilities["capabilities"]["spreed"]["config"]["federation"] = json!({
                "enabled": case != "disabled", "incoming-enabled": true,
            });
            let invitation = json!({"id":9,"localToken":"testroom", "userId": if case == "wrong-user" {"someone_else"} else {"fixture_user"},
                "roomName":"Fictional room", "inviterDisplayName":"Fictional inviter"});
            let mut replies = vec![Reply::data(capabilities.clone())];
            if case != "disabled" { replies.push(Reply::data(json!([invitation]))); }
            if matches!(case, "accept" | "decline") {
                replies.push(Reply::data(capabilities));
                let mut accepted = room();
                accepted["remoteServer"] = json!("https://remote.example.invalid");
                replies.push(Reply::data(if case == "accept" { accepted } else { Value::Null }));
            }
            let server = Server::start(replies).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            let list = client.request(context(&runtime), ProtocolRequest::FederationInvitations).await;
            if case == "wrong-user" {
                assert_eq!(list.err().unwrap().kind, ErrorKind::WrongScope);
                continue;
            }
            let ProtocolReply::FederationInvitations(view) = list.unwrap() else { panic!("invitations") };
            assert_eq!(view.supported, case != "disabled");
            if case == "disabled" { assert_eq!(server.count(), 1); continue; }
            let item = &view.items.as_slice()[0];
            assert_eq!(item.room.as_str(), "Fictional room");
            assert_ne!(item.id, 9);
            let operation = OperationId::from_local(44);
            let reply = client.request(context(&runtime), ProtocolRequest::ChangeFederationInvitation {
                operation, invitation: item.id, accept: case == "accept",
            }).await.unwrap();
            assert!(matches!(reply, ProtocolReply::FederationInvitationChanged { operation: got, invitation } if got == operation && invitation == item.id));
            let seen = server.seen.lock().unwrap();
            let request = String::from_utf8_lossy(&seen[3]);
            assert!(request.starts_with(if case == "accept" { "POST " } else { "DELETE " }));
            assert!(request.contains("/ocs/v2.php/apps/spreed/api/v1/federation/invitation/9"));
        }
    }
}
