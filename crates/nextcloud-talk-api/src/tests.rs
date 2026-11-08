use super::*;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[test]
fn marking_the_first_message_unread_uses_talks_explicit_sentinel() {
    let room = RoomToken::parse("room1234").unwrap();
    let (method, route, _, form, _, mutation) =
        plan(&Request::MarkUnreadFromStart { room }).unwrap();
    assert_eq!(method, Method::POST);
    assert!(route.ends_with("/chat/room1234/read"));
    assert_eq!(form, vec![("lastReadMessage", "-2".into())]);
    assert!(mutation);
}

#[tokio::test]
async fn empty_unread_forward_page_is_not_a_redirect() {
    let server = Server::start(vec![(304, String::new(), Duration::ZERO)]).await;
    let (mut client, clock) = client(&server);
    let response = client
        .request(
            &context(&clock),
            Request::Chat {
                room: RoomToken::parse("room1234").unwrap(),
                command: chat::Command::UnreadAfter { after: Some(42) },
            },
        )
        .await
        .unwrap();
    assert!(
        matches!(response, Reply::History { messages, last_given: None, .. } if messages.is_empty())
    );
}

#[test]
fn subscribed_threads_are_account_scoped_and_paged() {
    let (method, route, query, _, _, mutation) =
        plan(&Request::SubscribedThreads { offset: 50 }).unwrap();
    assert_eq!(method, Method::GET);
    assert_eq!(
        route,
        "ocs/v2.php/apps/spreed/api/v1/chat/subscribed-threads"
    );
    assert_eq!(query, vec![("limit", "50".into()), ("offset", "50".into())]);
    assert!(!mutation);
}

#[tokio::test]
async fn reference_thumbnails_cannot_fetch_a_third_party_or_unreviewed_server_route() {
    let server = Server::start(vec![]).await;
    let (mut owner, clock) = client(&server);
    for url in [
        "https://third-party.invalid/core/references/preview/123",
        "http://127.0.0.1:1/cloud/unreviewed/secret",
    ] {
        assert!(
            owner
                .reference_thumbnail(&context(&clock), url)
                .await
                .is_err()
        );
    }
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn notification_204_is_scoped_unsupported_and_empty_lists_remain_successful() {
    let account = || reply(json!({"id":"synthetic-user","display-name":"Fixture"}));
    let capabilities = || {
        reply(json!({"version":{"major":34},"capabilities":{
        "spreed":{"features":["conversation-v4","call-notification-state-api"]},
        "notifications":{"ocs-endpoints":["list","user-status"]}}}))
    };
    for calls in [false, true] {
        let server = Server::start(vec![
            account(),
            capabilities(),
            (204, String::new(), Duration::ZERO),
            account(),
        ])
        .await;
        let (mut owner, clock) = client(&server);
        let error = if calls {
            owner
                .call_invitations(&context(&clock))
                .await
                .err()
                .unwrap()
        } else {
            owner
                .message_notifications(&context(&clock))
                .await
                .err()
                .unwrap()
        };
        assert_eq!(error.kind, ErrorKind::Unsupported);
        assert_eq!(server.count(), 3);
        assert!(matches!(
            owner.request(&context(&clock), Request::Account).await,
            Ok(Reply::Account(_))
        ));
    }
    let server = Server::start(vec![account(), capabilities(), reply(json!([]))]).await;
    let (mut owner, clock) = client(&server);
    let list = owner.message_notifications(&context(&clock)).await.unwrap();
    assert!(list.messages.is_empty());
    assert_eq!(list.high_water, 0);
    for calls in [false, true] {
        let mut unexpected = reply(json!([]));
        unexpected.0 = 201;
        let server = Server::start(vec![account(), capabilities(), unexpected]).await;
        let (mut owner, clock) = client(&server);
        let error = if calls {
            owner
                .call_invitations(&context(&clock))
                .await
                .err()
                .unwrap()
        } else {
            owner
                .message_notifications(&context(&clock))
                .await
                .err()
                .unwrap()
        };
        assert_eq!(error.kind, ErrorKind::Malformed);
    }
    let server = Server::start(vec![(204, String::new(), Duration::ZERO)]).await;
    let (mut owner, clock) = client(&server);
    assert!(
        matches!(owner.request(&context(&clock), Request::Account).await, Err(error) if error.kind == ErrorKind::Malformed)
    );
}

#[tokio::test]
async fn normal_call_invitation_is_owner_bound_and_checks_current_status() {
    let mut missed = reply(json!([]));
    missed.0 = 201;
    let server=Server::start(vec![
        reply(json!({"id":"synthetic-user","display-name":"Fixture"})),
        reply(json!({"version":{"major":34},"capabilities":{"spreed":{"features":["conversation-v4","call-notification-state-api"]},"notifications":{"ocs-endpoints":["list","user-status"]}}})),
        reply(json!([{"notification_id":77,"app":"spreed","user":"synthetic-user","object_type":"call","object_id":"testroom","shouldNotify":true,"actions":[{"link":"https://unrelated.invalid"}]}])),
        reply(json!([])),missed,reply(json!([])),
    ]).await;
    let (mut owner, clock) = client(&server);
    let mut invitations = owner.call_invitations(&context(&clock)).await.unwrap();
    assert_eq!(invitations.len(), 1);
    let invitation = invitations.remove(0);
    // Missing source DND header cannot authorize an OS alert.
    assert!(!invitation.eligible());
    assert_eq!(invitation.id(), 77);
    let (mut other, other_clock) = client(&server);
    assert!(
        matches!(other.invitation_current(&context(&other_clock),&invitation).await,Err(error) if error.kind==ErrorKind::WrongScope)
    );
    assert_eq!(server.count(), 3);
    assert!(
        owner
            .invitation_current(&context(&clock), &invitation)
            .await
            .unwrap()
    );
    assert!(
        !owner
            .invitation_current(&context(&clock), &invitation)
            .await
            .unwrap()
    );
    owner
        .decline_invitation(&context(&clock), invitation)
        .await
        .unwrap();
    let requests = server.seen.lock().unwrap();
    assert_eq!(requests.len(), 6);
    let target = requests
        .last()
        .unwrap()
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(target[0], "DELETE");
    assert_eq!(
        target[1].split('?').next(),
        Some("/ocs/v2.php/apps/notifications/api/v2/notifications/77")
    );
    assert!(!requests.iter().any(|r| r.contains("unrelated.invalid")));
}
pub(super) struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
    job: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.job.abort();
    }
}
impl Server {
    pub(super) async fn start(replies: Vec<(u16, String, Duration)>) -> Self {
        Self::with_content_type(replies, "Content-Type: application/json\r\n").await
    }
    async fn with_content_type(
        replies: Vec<(u16, String, Duration)>,
        content_type: &'static str,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let out = seen.clone();
        let job = tokio::spawn(async move {
            for (status, body, delay) in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = stream.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    assert!(bytes.len() < 300000);
                    if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..end]).to_lowercase();
                        let length = header
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                out.lock().unwrap().push(String::from_utf8(bytes).unwrap());
                tokio::time::sleep(delay).await;
                let response = format!(
                    "HTTP/1.1 {status} fixture\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        Self { origin, seen, job }
    }
    pub(super) fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
    pub(super) fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}
pub(super) fn reply(data: Value) -> (u16, String, Duration) {
    (
        200,
        json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":data}}).to_string(),
        Duration::ZERO,
    )
}
pub(super) fn scope() -> SessionScope {
    SessionScope {
        account: BindingId::from_key(1),
        runtime: LeaseId::from_key(1),
    }
}
pub(super) fn client(server: &Server) -> (Client, NetworkRuntime) {
    let runtime = NetworkRuntime::new(ClockId::from_key(1));
    let client = Client::new(
        AppCredential::new(
            ServerOrigin::loopback(&server.origin).unwrap(),
            Zeroizing::new("synthetic-user".into()),
            Zeroizing::new("synthetic-app-password".into()),
            Some(Zeroizing::new("synthetic-user".into())),
        ),
        runtime.clone(),
        scope(),
        OriginId::from_key(1),
        Cancellation::default(),
    )
    .unwrap();
    (client, runtime)
}
pub(super) fn context(r: &NetworkRuntime) -> OperationContext {
    OperationContext {
        scope: scope(),
        deadline: Moment {
            epoch: r.now().epoch,
            millis: r.now().millis + 6000,
        },
        cancellation: Cancellation::default(),
    }
}

#[tokio::test]
async fn definitive_activation_and_join_denials_retire_only_provisional_state() {
    let denial = (
        403,
        "<html>temporary proxy denial</html>".into(),
        Duration::ZERO,
    );
    let server = Server::start(vec![
        denial.clone(),
        reply(json!({"sessionId":"own-session"})),
        denial,
        reply(json!([])),
    ])
    .await;
    let (mut client, runtime) = client(&server);
    assert!(
        matches!(client.request(&context(&runtime), Request::ActivateRoom(room())).await,
        Err(error) if error.kind == ErrorKind::ServerAccessDenied)
    );
    assert!(client.active_room.is_none());
    let other = RoomToken::parse("otherroom").unwrap();
    client
        .request(&context(&runtime), Request::ActivateRoom(other.clone()))
        .await
        .unwrap();
    let join = || Request::JoinCall {
        room: other.clone(),
        flags: models::ParticipantFlags::new(false, false),
        silent: true,
        recording_consent: false,
    };
    assert!(matches!(client.request(&context(&runtime), join()).await,
        Err(error) if error.kind == ErrorKind::ServerAccessDenied));
    assert!(!client.call_remote_pending);
    assert!(matches!(
        client.request(&context(&runtime), join()).await,
        Ok(Reply::CallJoined(_))
    ));
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn oversized_encoded_write_is_rejected_before_dispatch_without_uncertainty() {
    let server = Server::start(vec![]).await;
    let (mut client, runtime) = client(&server);
    let error = client
        .request(
            &context(&runtime),
            Request::Chat {
                room: room(),
                command: chat::Command::Send {
                    message: "&".repeat(90_000),
                    reference: "oversized-form".into(),
                    options: Default::default(),
                },
            },
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind, ErrorKind::TooLarge);
    assert_eq!(server.count(), 0);
    assert_eq!(
        client.sends.len(),
        1,
        "wire-size rejection follows journal admission"
    );
    assert!(client.sends.iter().all(|attempt| !attempt.uncertain));
}
pub(super) fn room() -> RoomToken {
    RoomToken::parse("testroom").unwrap()
}
fn room_data() -> Value {
    json!({"id":7,"token":"testroom","displayName":"Fixture"})
}
fn join_replies() -> Vec<(u16, String, Duration)> {
    vec![
        reply(room_data()),
        reply(json!({"sessionId":"own-session"})),
        reply(json!([])),
    ]
}
async fn join(client: &mut Client, runtime: &NetworkRuntime) -> signaling::CallHandle {
    client
        .request(&context(runtime), Request::Room(room()))
        .await
        .unwrap();
    client
        .request(&context(runtime), Request::ActivateRoom(room()))
        .await
        .unwrap();
    let Reply::CallJoined(handle) = client
        .request(
            &context(runtime),
            Request::JoinCall {
                room: room(),
                flags: ParticipantFlags::new(true, false),
                silent: true,
                recording_consent: false,
            },
        )
        .await
        .unwrap()
    else {
        panic!("call handle")
    };
    handle
}
fn signal_row(to: &str) -> Value {
    json!({"type":"message","data":json!({"from":"peer-session","to":to,"sid":"negotiation-one","roomType":"video","type":"offer","payload":{"type":"offer","sdp":"v=0\r\na=fixture\r\n"}}).to_string()})
}
#[tokio::test]
async fn meeting_protocol_uses_typed_signals_and_retires_stale_handle() {
    let mut replies = join_replies();
    replies.extend([
        reply(json!([])),
        reply(json!([signal_row("own-session")])),
        reply(json!([])),
    ]);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    let peer = signaling::PeerSession::parse("peer-session").unwrap();
    let negotiation = signaling::Negotiation::parse("negotiation-one").unwrap();
    client
        .send_signal(
            &context(&runtime),
            &handle,
            &peer,
            &negotiation,
            signaling::Signal::Description(
                signaling::SessionDescription::new(
                    signaling::DescriptionKind::Answer,
                    "v=0\r\na=answer\r\n",
                )
                .unwrap(),
            ),
        )
        .await
        .unwrap();
    let batch = client
        .poll_signals(&context(&runtime), &handle)
        .await
        .unwrap();
    assert_eq!(batch.events().len(), 1);
    let signaling::Event::Signal {
        stream: signaling::StreamKind::Camera,
        from,
        negotiation: received,
        signal: signaling::Signal::Description(sdp),
    } = &batch.events()[0]
    else {
        panic!("typed signal")
    };
    assert!(from.matches(&peer));
    assert!(received.matches(&negotiation));
    assert_eq!(sdp.kind(), signaling::DescriptionKind::Offer);
    client
        .request(&context(&runtime), Request::LeaveCall(room()))
        .await
        .unwrap();
    assert!(matches!(
        client.poll_signals(&context(&runtime), &handle).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert_eq!(server.count(), 6);
    let seen = server.seen.lock().unwrap();
    assert!(seen[2].starts_with("POST /ocs/v2.php/apps/spreed/api/v4/call/testroom?format=json"));
    assert!(seen[2].contains("flags=3&silent=true"));
    assert!(seen[3].contains("messages="));
    assert!(seen[3].to_lowercase().contains("ocs-apirequest: true"));
    assert!(
        !format!(
            "{:?}",
            signaling::IceCandidate::new(
                "video",
                0,
                "candidate:CANARY 1 UDP 1 127.0.0.1 9 typ host"
            )
            .unwrap()
        )
        .contains("CANARY")
    );
}
#[tokio::test]
async fn malformed_late_signal_yields_no_prefix_and_prevents_repoll() {
    let mut replies = join_replies();
    replies.push(reply(json!([
        signal_row("own-session"),
        signal_row("other-session")
    ])));
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    assert!(matches!(
        client.poll_signals(&context(&runtime), &handle).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert!(matches!(
        client.poll_signals(&context(&runtime), &handle).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert_eq!(server.count(), 4);
}
#[tokio::test]
async fn destructive_signal_poll_has_distinct_bounded_wait() {
    let mut replies = join_replies();
    let mut poll = reply(json!([]));
    poll.2 = Duration::from_millis(3100);
    replies.push(poll);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    assert!(
        client
            .poll_signals(&context(&runtime), &handle)
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn dropped_write_keeps_api_reference_uncertain_without_replay() {
    let mut response = reply(json!({}));
    response.2 = Duration::from_millis(200);
    let server = Server::start(vec![response]).await;
    let (mut client, runtime) = client(&server);
    let c = context(&runtime);
    {
        let pending = client.request(
            &c,
            Request::SendMessage {
                room: room(),
                message: "hello".into(),
                reference: "single-write".into(),
            },
        );
        tokio::pin!(pending);
        tokio::select! {_=&mut pending=>panic!("write settled early"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
    assert_eq!(server.count(), 1);
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::SendMessage {
                    room: room(),
                    message: "hello".into(),
                    reference: "single-write".into()
                }
            )
            .await,
        Err(Error {
            kind: ErrorKind::Uncertain,
            ..
        })
    ));
    assert_eq!(server.count(), 1);
}
#[tokio::test]
async fn scope_and_cancellation_fail_before_http() {
    let server = Server::start(vec![]).await;
    let (mut client, runtime) = client(&server);
    let mut c = context(&runtime);
    c.scope.runtime = LeaseId::from_key(2);
    assert!(matches!(
        client.request(&c, Request::Rooms).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    c.scope = scope();
    c.cancellation.cancel();
    assert!(matches!(
        client.request(&c, Request::Rooms).await,
        Err(Error {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert_eq!(server.count(), 0);
}
#[test]
fn typed_models_preserve_unknowns_and_strict_canonical_identity() {
    assert!(matches!(
        models::Account::decode(&json!({"id":"x".repeat(257)})),
        Err(ErrorKind::Malformed)
    ));
    assert_eq!(
        models::Room::decode(&json!({"id":"7","token":"testroom","displayName":"Fixture"}))
            .unwrap()
            .server_id,
        None
    );
    for major in [34, 35, 36] {
        let caps=models::Capabilities::decode(&json!({"version":{"major":major},"capabilities":{"spreed":{"features":["conversation-v4","future-feature"],"config":{"call":{"enabled":true}}}}})).unwrap();
        assert_eq!(caps.server_major, Some(major));
        assert!(caps.supports("future-feature"));
        assert_eq!(caps.encryption_required, None);
    }
}
#[test]
fn call_reactions_follow_bounded_server_configuration() {
    let value = |reactions| {
        json!({"version":{"major":35},"capabilities":{"spreed":{
            "features":["conversation-v4"],"config":{"call":{"supported-reactions":reactions}}
        }}})
    };
    let caps = models::Capabilities::decode(&value(json!(["👍", "🎉"]))).unwrap();
    assert_eq!(
        caps.call_reactions,
        Some(vec!["👍".to_owned(), "🎉".to_owned()])
    );
    for invalid in [
        json!([""]),
        json!(["x".repeat(65)]),
        json!(vec!["👍"; 33]),
        json!(["\n"]),
    ] {
        let caps = models::Capabilities::decode(&value(invalid)).unwrap();
        assert_eq!(caps.call_reactions, None);
        assert!(caps.supported_protocol());
    }
}
#[tokio::test]
async fn ringing_requires_fresh_call_permission_and_absent_local_user() {
    for case in [
        "allowed",
        "revoked",
        "guest",
        "already-in-call",
        "caller-left",
    ] {
        let mut replies = join_replies();
        replies.push(reply(json!({"version":{"major":35},"capabilities":{"spreed":{
            "features":["conversation-v4","send-call-notification"],"config":{"call":{"enabled":true}}
        }}})));
        replies.push(reply(
            json!({"id":7,"token":"testroom","displayName":"Fixture","readOnly":0,
            "hasCall":true,"permissions":if case == "revoked" {4} else {6}}),
        ));
        replies.push(reply(json!([{"attendeeId":44,"actorType":if case == "guest" {"guests"} else {"users"},
            "actorId":"other-user","displayName":"Other","inCall":if case == "already-in-call" {1} else {0},"sessionIds":["other-session"]}])));
        replies.push(reply(if case == "caller-left" {
            json!([])
        } else {
            json!([{"sessionId":"own-session"}])
        }));
        replies.push(reply(json!([])));
        let server = Server::start(replies).await;
        let (mut client, runtime) = client(&server);
        let handle = join(&mut client, &runtime).await;
        let (_reader, mut writer, _stop) = client.open_meeting(&context(&runtime), handle).unwrap();
        let mut moderation = writer.take_moderation_writer().unwrap();
        let result = moderation.ring(&context(&runtime), 44).await;
        if case == "allowed" {
            assert!(result.is_ok());
            assert!(server.seen.lock().unwrap().iter().any(|request| {
                request.starts_with("POST /ocs/v2.php/apps/spreed/api/v4/call/testroom/ring/44?")
            }));
        } else {
            assert!(result.is_err(), "{case}");
            assert!(!server.seen.lock().unwrap().iter().any(|request| {
                request.starts_with("POST /ocs/v2.php/apps/spreed/api/v4/call/testroom/ring/")
            }));
        }
    }
}
#[tokio::test]
async fn moderator_mute_targets_only_current_other_call_sessions() {
    for case in ["allowed", "member", "target-left", "caller-left"] {
        let mut replies = join_replies();
        replies.push(reply(
            json!({"version":{"major":35},"capabilities":{"spreed":{
                "features":["conversation-v4","force-mute"],"config":{"call":{"enabled":true}}
            }}}),
        ));
        replies.push(reply(
            json!({"id":7,"token":"testroom","displayName":"Fixture","readOnly":0,
            "hasCall":true,"participantType":if case == "member" {3} else {2}}),
        ));
        let mut participants = Vec::new();
        if case != "caller-left" {
            participants.push(json!({"sessionId":"own-session"}));
        }
        if case != "target-left" {
            participants.push(json!({"sessionId":"other-session"}));
        }
        replies.push(reply(json!(participants)));
        replies.push(reply(json!([])));
        let server = Server::start(replies).await;
        let (mut client, runtime) = client(&server);
        let handle = join(&mut client, &runtime).await;
        let (_reader, mut writer, _stop) = client.open_meeting(&context(&runtime), handle).unwrap();
        let mut moderation = writer.take_moderation_writer().unwrap();
        let target = signaling::PeerSession::parse("other-session").unwrap();
        let result = moderation
            .mute_participant(&context(&runtime), &target)
            .await;
        let sent = server.seen.lock().unwrap().iter().any(|request| {
            request.starts_with("POST /ocs/v2.php/apps/spreed/api/v3/signaling/testroom?")
                && request.contains("forceMute")
        });
        assert_eq!(result.is_ok(), case == "allowed", "{case}");
        assert_eq!(sent, case == "allowed", "{case}");
    }
}
#[tokio::test]
async fn recording_control_requires_fresh_backend_role_room_and_session() {
    for case in [
        "start",
        "stop",
        "no-backend",
        "no-capability",
        "classified",
        "member",
        "wrong-state",
        "caller-left",
    ] {
        let start = case != "stop";
        let mut replies = join_replies();
        replies.push(reply(json!({"version":{"major":35},"capabilities":{"spreed":{
            "features":if case == "no-capability" { vec!["conversation-v4"] } else { vec!["conversation-v4", "recording-v1"] },
            "config":{"call":{"enabled":true,"recording":case != "no-backend"}}
        }}})));
        replies.push(reply(
            json!({"id":7,"token":"testroom","displayName":"Fixture","readOnly":0,
            "hasCall":true,"participantType":if case == "member" {3} else {2},
            "attributes":if case == "classified" {4} else {0},
            "callRecording":if case == "stop" || case == "wrong-state" {1} else {0}}),
        ));
        replies.push(reply(if case == "caller-left" {
            json!([])
        } else {
            json!([{"sessionId":"own-session"}])
        }));
        replies.push(reply(json!([])));
        let server = Server::start(replies).await;
        let (mut client, runtime) = client(&server);
        let handle = join(&mut client, &runtime).await;
        let (_reader, mut writer, _stop) = client.open_meeting(&context(&runtime), handle).unwrap();
        let mut moderation = writer.take_moderation_writer().unwrap();
        let result = moderation.set_recording(&context(&runtime), start).await;
        let requests = server.seen.lock().unwrap();
        let sent = requests
            .iter()
            .find(|request| request.contains("/api/v1/recording/testroom?"));
        assert_eq!(result.is_ok(), matches!(case, "start" | "stop"), "{case}");
        assert_eq!(sent.is_some(), matches!(case, "start" | "stop"), "{case}");
        if let Some(request) = sent {
            if start {
                assert!(request.starts_with("POST ") && request.contains("status=1"));
            } else {
                assert!(request.starts_with("DELETE "));
            }
        }
    }
}
#[tokio::test]
async fn end_for_everyone_requires_fresh_moderator_and_current_session() {
    for case in ["allowed", "member", "breakout", "caller-left"] {
        let mut replies = join_replies();
        replies.push(reply(
            json!({"version":{"major":35},"capabilities":{"spreed":{
                "features":["conversation-v4"],"config":{"call":{"enabled":true}}
            }}}),
        ));
        replies.push(reply(
            json!({"id":7,"token":"testroom","displayName":"Fixture","readOnly":0,
            "hasCall":true,"participantType":if case == "member" {3} else {2},
            "objectType":if case == "breakout" {"room"} else {""}}),
        ));
        replies.push(reply(if case == "caller-left" {
            json!([])
        } else {
            json!([{"sessionId":"own-session"}])
        }));
        replies.push(reply(json!([])));
        let server = Server::start(replies).await;
        let (mut client, runtime) = client(&server);
        let handle = join(&mut client, &runtime).await;
        let (_reader, mut writer, _stop) = client.open_meeting(&context(&runtime), handle).unwrap();
        let mut moderation = writer.take_moderation_writer().unwrap();
        let result = moderation.end_for_everyone(&context(&runtime)).await;
        let sent = server.seen.lock().unwrap().iter().any(|request| {
            request.starts_with("DELETE /ocs/v2.php/apps/spreed/api/v4/call/testroom?")
                && request.contains("all=true")
        });
        assert_eq!(result.is_ok(), case == "allowed", "{case}");
        assert_eq!(sent, case == "allowed", "{case}");
    }
}
#[tokio::test]
async fn call_handle_cannot_cross_clients_even_with_equal_caller_keys() {
    let server_a = Server::start(join_replies()).await;
    let server_b = Server::start(join_replies()).await;
    let (mut a, ar) = client(&server_a);
    let (mut b, br) = client(&server_b);
    let handle_a = join(&mut a, &ar).await;
    let _handle_b = join(&mut b, &br).await;
    assert!(matches!(
        b.poll_signals(&context(&br), &handle_a).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert_eq!(server_b.count(), 3);
}
#[tokio::test]
async fn dropped_join_stays_uncertain_until_explicit_leave() {
    let mut replies = vec![
        reply(room_data()),
        reply(json!({"sessionId":"own-session"})),
    ];
    let mut delayed = reply(json!([]));
    delayed.2 = Duration::from_millis(200);
    replies.push(delayed);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    client
        .request(&context(&runtime), Request::Room(room()))
        .await
        .unwrap();
    client
        .request(&context(&runtime), Request::ActivateRoom(room()))
        .await
        .unwrap();
    let c = context(&runtime);
    {
        let join = client.request(
            &c,
            Request::JoinCall {
                room: room(),
                flags: ParticipantFlags::new(false, false),
                silent: true,
                recording_consent: false,
            },
        );
        tokio::pin!(join);
        tokio::select! {_=&mut join=>panic!("join settled early"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::JoinCall {
                    room: room(),
                    flags: ParticipantFlags::new(false, false),
                    silent: true,
                    recording_consent: false,
                }
            )
            .await,
        Err(Error {
            kind: ErrorKind::Uncertain,
            ..
        })
    ));
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn call_flags_and_other_room_leave_preserve_only_the_current_session() {
    let mut replies = join_replies();
    replies.extend([
        reply(json!([])), // flags update
        reply(json!([])), // leave a different conversation
        reply(json!([])), // current handle remains usable
        reply(json!({"sessionId":"replacement-session"})),
    ]);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    client
        .request(
            &context(&runtime),
            Request::UpdateCallFlags {
                room: room(),
                flags: ParticipantFlags::new(false, true),
            },
        )
        .await
        .unwrap();
    client
        .request(
            &context(&runtime),
            Request::LeaveRoom(RoomToken::parse("otherroom").unwrap()),
        )
        .await
        .unwrap();
    client
        .poll_signals(&context(&runtime), &handle)
        .await
        .unwrap();
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::DeactivateRoom(RoomToken::parse("otherroom").unwrap())
            )
            .await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    client
        .request(&context(&runtime), Request::ActivateRoom(room()))
        .await
        .unwrap();
    assert!(matches!(
        client.poll_signals(&context(&runtime), &handle).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert_eq!(server.count(), 7);
    let seen = server.seen.lock().unwrap();
    assert!(seen[3].starts_with("PUT /ocs/v2.php/apps/spreed/api/v4/call/testroom?format=json"));
    assert!(seen[3].contains("flags=5"));
}

#[tokio::test]
async fn dropped_call_flag_update_retires_handle_and_explicit_leave_still_works() {
    let mut replies = join_replies();
    let mut delayed = reply(json!([]));
    delayed.2 = Duration::from_millis(150);
    replies.extend([delayed, reply(json!([]))]);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    let c = context(&runtime);
    {
        let update = client.request(
            &c,
            Request::UpdateCallFlags {
                room: room(),
                flags: ParticipantFlags::new(false, false),
            },
        );
        tokio::pin!(update);
        tokio::select! {_=&mut update=>panic!("update settled early"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
    assert!(matches!(
        client.poll_signals(&context(&runtime), &handle).await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::JoinCall {
                    room: room(),
                    flags: ParticipantFlags::new(false, false),
                    silent: true,
                    recording_consent: false,
                }
            )
            .await,
        Err(Error {
            kind: ErrorKind::Uncertain,
            ..
        })
    ));
    client
        .request(&context(&runtime), Request::LeaveCall(room()))
        .await
        .unwrap();
    assert_eq!(server.count(), 5);
}

#[tokio::test]
async fn dropped_activation_cannot_join_or_switch_rooms_before_cleanup() {
    let mut delayed = reply(json!({"sessionId":"late-session"}));
    delayed.2 = Duration::from_millis(150);
    let server = Server::start(vec![delayed, reply(json!([]))]).await;
    let (mut client, runtime) = client(&server);
    let c = context(&runtime);
    {
        let activate = client.request(&c, Request::ActivateRoom(room()));
        tokio::pin!(activate);
        tokio::select! {_=&mut activate=>panic!("activation settled early"),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
    }
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::JoinCall {
                    room: room(),
                    flags: ParticipantFlags::new(false, false),
                    silent: true,
                    recording_consent: false,
                }
            )
            .await,
        Err(Error {
            kind: ErrorKind::WrongScope,
            ..
        })
    ));
    assert!(matches!(
        client
            .request(
                &context(&runtime),
                Request::ActivateRoom(RoomToken::parse("otherroom").unwrap())
            )
            .await,
        Err(Error {
            kind: ErrorKind::Busy,
            ..
        })
    ));
    client
        .request(&context(&runtime), Request::DeactivateRoom(room()))
        .await
        .unwrap();
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn unstructured_http_refusals_preserve_safe_causes_without_claiming_revocation() {
    for content_type in [
        "",
        "Content-Type: text/html\r\n",
        "Content-Type: application/json\r\n",
    ] {
        for (status, expected) in [
            (403, ErrorKind::ServerAccessDenied),
            (503, ErrorKind::ServerUnavailable),
            (401, ErrorKind::Malformed),
        ] {
            let server = Server::with_content_type(
                vec![(status, "Forbidden".into(), Duration::ZERO)],
                content_type,
            )
            .await;
            let (mut owner, clock) = client(&server);
            let result = owner.request(&context(&clock), Request::Account).await;
            assert!(matches!(result, Err(error) if error.kind == expected));
            assert_eq!(server.count(), 1, "a refused request must not be retried");
        }
    }
    // A valid OCS room refusal keeps its permission meaning rather than turning
    // a conversation restriction into an account-wide connectivity failure.
    let server = Server::start(vec![(
        403,
        json!({"ocs":{"meta":{"status":"failure","statuscode":403,"message":"denied"},"data":[]}})
            .to_string(),
        Duration::ZERO,
    )])
    .await;
    let (mut owner, clock) = client(&server);
    assert!(
        matches!(owner.request(&context(&clock), Request::Rooms).await, Err(error) if error.kind == ErrorKind::PermissionDenied)
    );
}

#[test]
fn internal_screen_offer_and_payload_free_stop_preserve_camera_batch() {
    use signaling::{Event, StreamKind};
    let msg = |kind: &str, stream: &str| json!({"type":"message","data":json!({"to":"local","from":"remote","sid":"separate-screen","roomType":stream,"type":kind,"payload":{"type":kind,"sdp":"v=0\r\n"}}).to_string()});
    let stop = json!({"type":"message","data":json!({"to":"local","from":"remote","sid":"separate-screen","roomType":"screen","type":"unshareScreen"}).to_string()});
    let batch = signaling::parse_batch(
        &json!([msg("offer", "screen"), stop, msg("answer", "video")]),
        "local",
        1,
    )
    .unwrap();
    assert!(matches!(
        batch.events()[0],
        Event::Signal {
            stream: StreamKind::Screen,
            ..
        }
    ));
    assert!(matches!(batch.events()[1], Event::ScreenStopped { .. }));
    assert!(matches!(
        batch.events()[2],
        Event::Signal {
            stream: StreamKind::Camera,
            ..
        }
    ));
}

#[tokio::test]
async fn internal_poll_failure_reports_interruption_then_resume_without_new_authority() {
    let mut replies = join_replies();
    replies.extend([
        (503, "unavailable".into(), Duration::ZERO),
        reply(json!([signal_row("own-session")])),
    ]);
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    let (mut reader, _writer, stop) = client.open_meeting(&context(&runtime), handle).unwrap();
    let failed = reader.poll(&context(&runtime)).await.unwrap();
    assert!(matches!(
        failed.events(),
        [signaling::Event::TransportInterrupted]
    ));
    assert!(!stop.is_closed());
    let resumed = reader.poll(&context(&runtime)).await.unwrap();
    assert!(matches!(
        resumed.events(),
        [
            signaling::Event::TransportResumed,
            signaling::Event::Signal { .. }
        ]
    ));
    drop(reader);
    assert!(
        stop.is_closed(),
        "abandoning a poll owner still closes the route"
    );
}

#[tokio::test]
async fn internal_poll_recovery_exhaustion_closes_route() {
    let mut replies = join_replies();
    replies.extend((0..4).map(|_| (503, "unavailable".into(), Duration::ZERO)));
    let server = Server::start(replies).await;
    let (mut client, runtime) = client(&server);
    let handle = join(&mut client, &runtime).await;
    let (mut reader, _writer, stop) = client.open_meeting(&context(&runtime), handle).unwrap();
    for _ in 0..3 {
        assert!(reader.poll(&context(&runtime)).await.is_ok());
        assert!(!stop.is_closed());
    }
    assert!(reader.poll(&context(&runtime)).await.is_err());
    assert!(stop.is_closed());
    assert_eq!(server.count(), 7);
}

#[tokio::test]
async fn conversation_links_are_bound_to_the_accounts_origin_and_installation_path() {
    let server = Server::start(vec![]).await;
    let runtime = NetworkRuntime::new(ClockId::from_key(1));
    let client = Client::new(
        AppCredential::new(
            ServerOrigin::loopback(&format!("{}/nextcloud/", server.origin)).unwrap(),
            Zeroizing::new("synthetic-user".into()),
            Zeroizing::new("synthetic-password".into()),
            None,
        ),
        runtime,
        scope(),
        OriginId::from_key(1),
        Cancellation::default(),
    )
    .unwrap();
    for path in [
        "nextcloud/call/testroom",
        "nextcloud/index.php/call/testroom",
    ] {
        assert!(
            client
                .conversation_link_token(&format!("{}/{path}", server.origin))
                .is_ok()
        );
    }
    for link in [
        format!("{}/call/testroom", server.origin),
        format!("{}/nextcloud/call/testroom?password=secret", server.origin),
        format!("{}/nextcloud/call/testroom#fragment", server.origin),
        "https://unrelated.invalid/nextcloud/call/testroom".into(),
        format!("{}/nextcloud/call/INVALID", server.origin),
    ] {
        assert!(client.conversation_link_token(&link).is_err());
    }
    assert_eq!(
        server.count(),
        0,
        "link parsing never contacts the supplied destination"
    );
}
