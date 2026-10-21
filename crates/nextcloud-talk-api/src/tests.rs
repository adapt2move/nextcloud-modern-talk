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
                    "HTTP/1.1 {status} fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
        Err(error) if error.kind == ErrorKind::PermissionDenied)
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
        Err(error) if error.kind == ErrorKind::PermissionDenied));
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
