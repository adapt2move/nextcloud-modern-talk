use super::*;

fn notification_caps() -> Value {
    let mut value = caps(35);
    value["capabilities"]["notifications"] = json!({"ocs-endpoints":["list","user-status"]});
    value
}
fn notifications(rows: Value, status: &str) -> Vec<Reply> {
    let mut list = Reply::data(rows);
    list.headers = format!("X-Nextcloud-User-Status: {status}\r\n");
    vec![
        Reply::data(json!({"id":"fixture_user","display-name":"Fixture"})),
        Reply::data(notification_caps()),
        list,
    ]
}
// The wire endpoint emits PHP's ISO-8601 `c` format. Avoid an extra test dependency.
fn wall_time() -> (u64, String) {
    // Small fixed server-clock lead avoids an accidental second boundary while
    // the initial baseline's three HTTP reads complete.
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 5;
    let mut days = seconds / 86400;
    let mut year = 1970;
    let leap = |y| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    loop {
        let length = if leap(year) { 366 } else { 365 };
        if days < length {
            break;
        }
        days -= length;
        year += 1;
    }
    let mut month = 1;
    for length in [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ] {
        if days < length {
            break;
        }
        days -= length;
        month += 1;
    }
    (
        seconds,
        format!(
            "{year:04}-{month:02}-{:02}T{:02}:{:02}:{:02}+00:00",
            days + 1,
            seconds / 3600 % 24,
            seconds / 60 % 60,
            seconds % 60
        ),
    )
}
fn notice(id: u64, object: &str, timestamp: &str) -> Value {
    json!({"notification_id":id,"app":"spreed","user":"fixture_user","object_type":"chat",
        "object_id":object,"shouldNotify":true,"datetime":timestamp,
        "subject":"Never interpreted","link":"https://unrelated.invalid/"})
}
fn message(id: u64, timestamp: u64) -> Value {
    // Pinned Talk omits false `silent` and an absent last edit.
    json!({"id":id,"token":"testroom","actorType":"users","actorId":"bob",
        "actorDisplayName":"Bob","messageType":"comment","message":"Background hello",
        "timestamp":timestamp,"systemMessage":"",
        "messageParameters":{},"reactions":{},"reactionsSelf":[]})
}
async fn poll(
    c: &mut NextcloudClient,
    r: &NetworkRuntime,
) -> talk_core::protocol::MessageNotifications {
    let ProtocolReply::MessageNotifications(value) = c
        .request(context(r), ProtocolRequest::PollMessageNotifications)
        .await
        .unwrap()
    else {
        panic!("message observation");
    };
    value
}

#[tokio::test]
async fn live_drain_is_exact_scope_and_cancellation_checked_before_io() {
    let server = Server::start(vec![Reply::data(json!({
        "signalingMode":"internal","stunservers":[],"turnservers":[]
    }))])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    for wrong_account in [true, false] {
        let mut ctx = context(&r);
        if wrong_account {
            ctx.scope.account = AccountId::from_local(9);
        } else {
            ctx.scope.runtime = RuntimeGeneration::from_local(9);
        }
        assert_eq!(
            c.request(ctx, ProtocolRequest::PollLiveEvents)
                .await
                .unwrap_err()
                .kind,
            ErrorKind::WrongScope
        );
    }
    let cancelled = context(&r);
    cancelled.cancellation.cancel();
    assert_eq!(
        c.request(cancelled, ProtocolRequest::PollLiveEvents)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(server.count(), 0);
    for _ in 0..2 {
        let ProtocolReply::LiveEvents(events) = c
            .request(context(&r), ProtocolRequest::PollLiveEvents)
            .await
            .unwrap()
        else {
            panic!("typed live drain");
        };
        assert_eq!(events.scope, scope());
        assert_eq!(
            events.state,
            talk_core::protocol::AccountLiveState::Internal
        );
        assert!(events.changes.as_slice().is_empty());
    }
    assert_eq!(server.count(), 1, "the existing API owner caches settings");
    let requests = server.seen.lock().unwrap();
    assert!(
        String::from_utf8_lossy(&requests[0])
            .starts_with("GET /cloud/ocs/v2.php/apps/spreed/api/v3/signaling/settings?")
    );
    assert!(c.active.is_none());
    assert!(c.meeting_owner.is_none());
}

#[tokio::test]
async fn live_internal_or_failure_preserves_http_notifications_without_replay_or_read_effects() {
    for failed in [false, true] {
        let (stamp, datetime) = wall_time();
        let event = notice(101, "testroom/42", &datetime);
        let mut replies = notifications(json!([notice(100, "testroom/41", &datetime)]), "online");
        replies.push(if failed {
            Reply::envelope(503, "failure", 503, json!([]))
        } else {
            Reply::data(json!({
                "signalingMode":"internal","stunservers":[],"turnservers":[]
            }))
        });
        replies.extend(notifications(json!([event.clone()]), "online"));
        replies.push(Reply::data(notification_caps()));
        let mut data = room();
        data["notificationLevel"] = json!(1);
        replies.push(Reply::data(data));
        replies.push(Reply::data(json!([message(42, stamp)])));
        replies.extend(notifications(json!([event]), "online"));
        let server = Server::start(replies).await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        assert!(poll(&mut c, &r).await.notices.as_slice().is_empty());
        let live = c
            .request(context(&r), ProtocolRequest::PollLiveEvents)
            .await;
        if failed {
            assert_eq!(live.unwrap_err().kind, ErrorKind::ServerUnavailable);
        } else {
            assert!(matches!(live, Ok(ProtocolReply::LiveEvents(_))));
        }
        assert_eq!(poll(&mut c, &r).await.notices.as_slice().len(), 1);
        assert!(poll(&mut c, &r).await.notices.as_slice().is_empty());
        assert!(c.active.is_none());
        assert!(c.meeting_owner.is_none());
        for request in server.seen.lock().unwrap().iter() {
            let request = String::from_utf8_lossy(request);
            assert!(request.starts_with("GET "));
            assert!(!request.contains("/read") && !request.contains("lookIntoFuture"));
            assert!(
                !request.contains("setReadMarker=1")
                    && !request.contains("markNotificationsAsRead=1")
            );
        }
    }
}

#[tokio::test]
async fn background_room_arrival_is_read_only_and_history_self_edits_mutes_and_replays_are_silent()
{
    for case in [
        "ordinary",
        "explicit-audible",
        "sensitive",
        "mute",
        "mention",
        "display-mention",
        "self",
        "edited",
        "silent",
        "null-silent",
        "malformed-silent",
        "dnd",
        "old",
        "reissued",
    ] {
        let (stamp, datetime) = wall_time();
        let mut data = room();
        data["notificationLevel"] = json!(1);
        let mut incoming = message(42, stamp);
        let object = if case == "sensitive" {
            "testroom"
        } else {
            "testroom/42"
        };
        let mut event = notice(101, object, &datetime);
        match case {
            "explicit-audible" => incoming["silent"] = json!(false),
            "sensitive" => data["isSensitive"] = json!(true),
            "mute" => data["notificationLevel"] = json!(3),
            "mention" => {
                data["notificationLevel"] = json!(2);
                incoming["message"] = json!("Hello {mention}");
                incoming["messageParameters"] =
                    json!({"mention":{"type":"user","id":"fixture_user","name":"Fixture"}});
            }
            "display-mention" => {
                data["notificationLevel"] = json!(2);
                incoming["message"] = json!("Hello @fixture_user");
            }
            "self" => incoming["actorId"] = json!("fixture_user"),
            "edited" => incoming["lastEditTimestamp"] = json!(stamp),
            "silent" => incoming["silent"] = json!(true),
            "null-silent" => incoming["silent"] = Value::Null,
            "malformed-silent" => incoming["silent"] = json!("false"),
            "old" => event["datetime"] = json!("2000-01-01T00:00:00+00:00"),
            _ => {}
        }
        let baseline = if case == "reissued" {
            "testroom/42"
        } else {
            "testroom/41"
        };
        let mut replies = notifications(json!([notice(100, baseline, &datetime)]), "online");
        replies.extend(notifications(
            json!([event.clone()]),
            if case == "dnd" { "dnd" } else { "online" },
        ));
        replies.push(Reply::data(notification_caps()));
        if !matches!(case, "dnd" | "old") {
            replies.push(Reply::data(data));
            if case != "mute" {
                replies.push(Reply::data(json!([incoming])));
            }
        }
        replies.extend(notifications(json!([event]), "online"));
        let server = Server::start(replies).await;
        let r = runtime();
        let mut c = client(&server.origin, r.clone());
        assert!(
            poll(&mut c, &r).await.notices.as_slice().is_empty(),
            "{case}: initial history"
        );
        let value = poll(&mut c, &r).await;
        let expected = matches!(
            case,
            "ordinary" | "explicit-audible" | "sensitive" | "mention"
        );
        assert_eq!(
            value.notices.as_slice().len(),
            usize::from(expected),
            "{case}"
        );
        assert!(
            value.changed,
            "{case}: navigation reconciles even when muted"
        );
        if case == "sensitive" {
            let alert = &value.notices.as_slice()[0];
            assert!(alert.redacted);
            assert!(alert.title.as_str().is_empty() && alert.body.as_str().is_empty());
        }
        assert!(
            poll(&mut c, &r).await.notices.as_slice().is_empty(),
            "{case}: replay"
        );
        assert!(
            c.active.is_none(),
            "{case}: never activate a room for an alert"
        );
        let requests = server.seen.lock().unwrap();
        for request in requests.iter() {
            let request = String::from_utf8_lossy(request);
            assert!(request.starts_with("GET "), "{case}");
            assert!(
                !request.contains("setReadMarker=1")
                    && !request.contains("markNotificationsAsRead=1")
            );
            assert!(!request.contains("unrelated.invalid"));
        }
        if case == "sensitive" {
            let history = requests
                .iter()
                .map(|v| String::from_utf8_lossy(v))
                .find(|v| v.contains("/chat/testroom?"))
                .unwrap();
            assert!(
                history.contains("setReadMarker=0")
                    && history.contains("markNotificationsAsRead=0")
            );
        }
    }
}

#[tokio::test]
async fn complete_lists_keep_metadata_and_stable_ids_but_partial_or_oversized_lists_fail() {
    let mut data = room();
    data["isFavorite"] = json!(true);
    data["isArchived"] = json!(true);
    data["unreadMessages"] = json!(7);
    let mut partial = Reply::data(json!([]));
    partial.status = 206;
    let mut capabilities = caps(35);
    capabilities["capabilities"]["spreed"]["features"]
        .as_array_mut()
        .unwrap()
        .extend([json!("archived-conversations-v2"), json!("favorites")]);
    let server = Server::start(vec![
        Reply::data(capabilities.clone()),
        Reply::data(json!([data.clone()])),
        Reply::data(capabilities.clone()),
        Reply::data(json!([data.clone()])),
        Reply::data(capabilities.clone()),
        partial,
        Reply::data(capabilities),
        Reply::data(json!(vec![data; 101])),
    ])
    .await;
    let r = runtime();
    let mut c = client(&server.origin, r.clone());
    let ProtocolReply::Conversations(first) = c
        .request(context(&r), ProtocolRequest::ListConversations)
        .await
        .unwrap()
    else {
        panic!("rooms");
    };
    let ProtocolReply::Conversations(second) = c
        .request(context(&r), ProtocolRequest::ListConversations)
        .await
        .unwrap()
    else {
        panic!("rooms");
    };
    assert_eq!(first, second);
    assert_eq!(first.rooms.as_slice()[0].archived, Some(true));
    assert_eq!(first.rooms.as_slice()[0].favorite, Some(true));
    assert_eq!(first.rooms.as_slice()[0].unread_messages, Some(7));
    assert_eq!(
        c.request(context(&r), ProtocolRequest::ListConversations)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Malformed
    );
    assert_eq!(
        c.request(context(&r), ProtocolRequest::ListConversations)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::TooLarge
    );
    assert_eq!(c.rooms.len(), 1);
}
