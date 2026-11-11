//! Finite local WebSocket fixtures qualify protocol ownership only. They are not
//! HPB authentication, Janus, ICE, policy or media interoperability evidence.
use super::*;
use crate::{
    AppCredential, ServerOrigin,
    scope::{BindingId, ClockId, LeaseId, OriginId},
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

#[tokio::test]
async fn explicit_call_admission_replaces_exhausted_transport_without_reviving_old_routes() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal/", listener.local_addr().unwrap());
    let (done, wait) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut cleanup = accept(&listener).await;
        hello(&mut cleanup, true).await;
        let bye = receive(&mut cleanup).await;
        assert_eq!(bye["type"], "bye");
        send(&mut cleanup, json!({"id":bye["id"],"type":"bye","bye":{}})).await;
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        join(&mut socket).await;
        let _ = wait.await;
    });
    let (mut client, binding, runtime) = account();
    let external = settings(&endpoint);
    let old = Arc::new(state(binding));
    *old.settings.lock().unwrap() = external.clone();
    *old.retirement_resume.lock().unwrap() = Some(session::Resume {
        session: peer("hpb-own"),
        secret: Zeroizing::new("opaque-resume-secret".into()),
    });
    old.status(LiveState::Stopped(ErrorKind::Transport));
    let (commands, receive) = mpsc::channel(1);
    drop(receive); // The exhausted socket task has completed.
    let handle = Handle {
        shared: old.clone(),
        commands,
    };
    let old_route = Route {
        handle: handle.clone(),
        generation: 1,
        cancellation: old
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .cancellation
            .clone(),
        replaced: Arc::new(AtomicBool::new(false)),
    };
    client.hpb = Some(Owner { handle });
    client.hpb_route = Some(old_route.clone());
    client.active_room = Some(room());
    client.room_session = Some(Zeroizing::new("talk-own".into()));
    let model = SignalingSettings {
        mode: SignalingMode::External,
        external: Some(external),
        ice_servers: Vec::new(),
    };
    let ctx = context(&runtime, &old);
    client.configure_hpb(&ctx, &model).unwrap();
    assert!(Arc::ptr_eq(
        &client.hpb.as_ref().unwrap().handle.shared,
        &old
    ));
    let cancelled = context(&runtime, &old);
    cancelled.cancellation.cancel();
    assert_eq!(
        client
            .prepare_external_meeting(&cancelled, &room(), &model)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert!(Arc::ptr_eq(
        &client.hpb.as_ref().unwrap().handle.shared,
        &old
    ));
    assert_eq!(
        client
            .prepare_external_meeting(&ctx, &room(), &model)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Uncertain
    );
    assert_eq!(
        client
            .retire_stopped_external_meeting(&cancelled, &room())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert!(old.retirement_resume.lock().unwrap().is_some());
    client
        .retire_stopped_external_meeting(&ctx, &room())
        .await
        .unwrap();
    assert!(client.hpb.is_none());
    assert!(old.state.lock().unwrap().room.is_none());
    assert!(old.retirement_resume.lock().unwrap().is_none());
    client
        .prepare_external_meeting(&ctx, &room(), &model)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(
        &client.hpb.as_ref().unwrap().handle.shared,
        &old
    ));
    assert!(old_route.is_closed());
    assert!(old.stop.is_cancelled());
    assert!(
        client
            .hpb_route
            .as_ref()
            .unwrap()
            .matches(&room(), &peer("talk-own"))
    );
    done.send(()).unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn a_refused_hello_is_tried_again_with_the_token_of_a_new_admission() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal/", listener.local_addr().unwrap());
    let (done, wait) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        join(&mut socket).await;
        let _ = wait.await;
    });
    let (mut client, binding, runtime) = account();
    let external = settings(&endpoint);
    let old = Arc::new(state(binding));
    *old.settings.lock().unwrap() = external.clone();
    old.state.lock().unwrap().room = None;
    old.status(LiveState::Stopped(ErrorKind::PermissionDenied));
    let (commands, receive) = mpsc::channel(1);
    drop(receive); // The refused session task has completed.
    client.hpb = Some(Owner {
        handle: Handle {
            shared: old.clone(),
            commands,
        },
    });
    client.active_room = Some(room());
    client.room_session = Some(Zeroizing::new("talk-own".into()));
    let model = SignalingSettings {
        mode: SignalingMode::External,
        external: Some(external),
        ice_servers: Vec::new(),
    };
    let ctx = context(&runtime, &old);
    client
        .prepare_external_meeting(&ctx, &room(), &model)
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(
        &client.hpb.as_ref().unwrap().handle.shared,
        &old
    ));
    assert!(old.stop.is_cancelled());
    // A session that still holds a room is retired first, never replaced.
    let (_, binding, _) = account();
    let held = Arc::new(state(binding));
    held.status(LiveState::Stopped(ErrorKind::PermissionDenied));
    let (commands, _receive) = mpsc::channel(1);
    let owner = Owner {
        handle: Handle {
            shared: held,
            commands,
        },
    };
    assert!(owner.hello_refused() && owner.retirement_pending());
    done.send(()).unwrap();
    server.await.unwrap();
}

#[test]
fn fresh_call_transport_retry_does_not_reinterpret_auth_or_policy_rejection() {
    let (_client, binding, _) = account();
    let shared = Arc::new(state(binding));
    let (commands, _receive) = mpsc::channel(1);
    let owner = Owner {
        handle: Handle {
            shared: shared.clone(),
            commands,
        },
    };
    for error in [
        ErrorKind::ReauthenticationRequired,
        ErrorKind::PermissionDenied,
        ErrorKind::WrongScope,
        ErrorKind::Unsupported,
        ErrorKind::Cancelled,
    ] {
        shared.status(LiveState::Stopped(error));
        assert!(!owner.transport_stopped());
    }
    for error in [ErrorKind::Transport, ErrorKind::Deadline] {
        shared.status(LiveState::Stopped(error));
        assert!(owner.transport_stopped());
    }
}

#[tokio::test]
async fn retirement_requires_matched_absence_or_bye_and_retains_rotated_resume_on_failure() {
    for outcome in [
        "absent",
        "wrong-correlation",
        "wrong-session",
        "unconfirmed-bye",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/signal/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            send(
                &mut socket,
                json!({"type":"welcome","welcome":{"features":[]}}),
            )
            .await;
            let request = receive(&mut socket).await;
            assert_eq!(request["type"], "hello");
            assert_eq!(request["hello"]["resumeid"], "old-resume");
            assert!(request["hello"].get("auth").is_none());
            match outcome {
                "absent" | "wrong-correlation" => {
                    let id = if outcome == "absent" {
                        request["id"].clone()
                    } else {
                        json!("wrong")
                    };
                    send(
                        &mut socket,
                        json!({"id":id,"type":"error","error":{"code":"no_such_session"}}),
                    )
                    .await;
                }
                _ => {
                    let session = if outcome == "wrong-session" {
                        "other-session"
                    } else {
                        "hpb-own"
                    };
                    send(&mut socket, json!({"id":request["id"],"type":"hello","hello":{"version":"1.0","sessionid":session,"resumeid":"rotated-resume"}})).await;
                    if outcome == "unconfirmed-bye" {
                        let bye = receive(&mut socket).await;
                        assert_eq!(bye["type"], "bye");
                        send(&mut socket, json!({"id":"wrong","type":"bye","bye":{}})).await;
                        // Closing the socket is not definitive session retirement.
                    }
                }
            }
        });
        let (_client, binding, runtime) = account();
        let shared = state(binding);
        *shared.settings.lock().unwrap() = settings(&endpoint);
        *shared.retirement_resume.lock().unwrap() = Some(session::Resume {
            session: peer("hpb-own"),
            secret: Zeroizing::new("old-resume".into()),
        });
        shared.status(LiveState::Stopped(ErrorKind::Transport));
        let result = session::retire_stopped(&shared, &context(&runtime, &shared)).await;
        server.await.unwrap();
        if outcome == "absent" {
            assert_eq!(result, Ok(()));
            assert!(shared.state.lock().unwrap().room.is_none());
            assert!(shared.retirement_resume.lock().unwrap().is_none());
        } else {
            assert!(result.is_err());
            assert!(shared.state.lock().unwrap().room.is_some());
            assert!(
                shared
                    .state
                    .lock()
                    .unwrap()
                    .room
                    .as_ref()
                    .unwrap()
                    .cancellation
                    .is_cancelled()
            );
            let retained = shared.retirement_resume.lock().unwrap();
            assert_eq!(
                retained.as_ref().unwrap().secret.as_str(),
                if outcome == "unconfirmed-bye" {
                    "rotated-resume"
                } else {
                    "old-resume"
                }
            );
        }
    }
}

type ServerSocket = WebSocketStream<TcpStream>;
fn settings(server: &str) -> ExternalSettings {
    ExternalSettings::decode(&json!({
        "server":server,"userId":"fixture-user","ticket":"opaque-v1-ticket",
        "helloAuthParams":{"2.0":{"token":"opaque-v2-token","extension":"preserved"}}
    }))
    .unwrap()
}
fn account() -> (Client, AccountBinding, NetworkRuntime) {
    let runtime = NetworkRuntime::new(ClockId::from_key(11));
    let client = Client::new(
        AppCredential::new(
            ServerOrigin::https("https://account.invalid/nc").unwrap(),
            Zeroizing::new("fixture-login".into()),
            Zeroizing::new("never-forward-password".into()),
            Some(Zeroizing::new("fixture-user".into())),
        ),
        runtime.clone(),
        SessionScope {
            account: BindingId::from_key(1),
            runtime: LeaseId::from_key(2),
        },
        OriginId::from_key(3),
        Cancellation::default(),
    )
    .unwrap();
    let binding = client.transport.hpb_binding().unwrap();
    (client, binding, runtime)
}
fn context(runtime: &NetworkRuntime, shared: &Shared) -> OperationContext {
    OperationContext {
        scope: shared.account.scope,
        deadline: Moment {
            epoch: runtime.now().epoch,
            millis: runtime.now().millis + 4000,
        },
        cancellation: Cancellation::default(),
    }
}
fn peer(value: &str) -> PeerSession {
    PeerSession::parse(value).unwrap()
}
fn room() -> RoomToken {
    RoomToken::parse("testroom").unwrap()
}
fn state(binding: AccountBinding) -> Shared {
    Shared {
        account: binding,
        stop: Cancellation::default(),
        settings: Mutex::new(settings("https://hpb.invalid")),
        retirement_resume: Mutex::new(None),
        state: Mutex::new(State {
            status: LiveState::Connected,
            first_transport_failure: None,
            events: Mailbox::default(),
            session: Some(peer("hpb-own")),
            mcu: true,
            generation: 1,
            room: Some(Room {
                purpose: RoomPurpose::Call,
                replaced: Arc::new(AtomicBool::new(false)),
                token: room(),
                own: peer("talk-own"),
                generation: 1,
                cancellation: Cancellation::default(),
                joined: true,
                members: Vec::new(),
                in_call: Vec::new(),
                subscription_requests: Vec::new(),
                selected: Vec::new(),
                retired_screens: Vec::with_capacity(32),
                retired_subscriptions: Vec::with_capacity(64),
                signals: VecDeque::new(),
                signal_bytes: 0,
                typing: Vec::new(),
            }),
        }),
    }
}
// The maintained handshake callback requires this concrete HTTP response type.
#[allow(clippy::result_large_err)]
async fn accept(listener: &TcpListener) -> ServerSocket {
    let (stream, _) = tokio::time::timeout(Duration::from_secs(3), listener.accept())
        .await
        .unwrap()
        .unwrap();
    tokio_tungstenite::accept_hdr_async(
        stream,
        |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
            assert_eq!(request.uri().path(), "/signal/spreed");
            assert_eq!(request.headers()["origin"], "https://account.invalid");
            assert!(request.headers().get("authorization").is_none());
            assert!(request.headers().get("cookie").is_none());
            Ok(response)
        },
    )
    .await
    .unwrap()
}
async fn receive(socket: &mut ServerSocket) -> Value {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(_) => socket.flush().await.unwrap(),
                _ => panic!("unexpected fixture frame"),
            }
        }
    })
    .await
    .unwrap()
}
async fn send(socket: &mut ServerSocket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn hello(socket: &mut ServerSocket, resume: bool) {
    send(socket, json!({"type":"welcome","welcome":{"features":["hello-v2","mcu","audio-video-permissions","incall-all"]}})).await;
    let request = receive(socket).await;
    assert_eq!(request["type"], "hello");
    assert!(!request.to_string().contains("never-forward-password"));
    if resume {
        assert_eq!(request["hello"]["resumeid"], "opaque-resume-secret");
        assert!(request["hello"].get("auth").is_none());
    } else {
        assert_eq!(request["hello"]["version"], "2.0");
        assert_eq!(
            request["hello"]["auth"]["url"],
            "https://account.invalid/nc/ocs/v2.php/apps/spreed/api/v3/signaling/backend"
        );
        assert_eq!(
            request["hello"]["auth"]["params"]["token"],
            "opaque-v2-token"
        );
        assert_eq!(request["hello"]["auth"]["params"]["extension"], "preserved");
    }
    send(socket, json!({"id":request["id"],"type":"hello","hello":{"version":"2.0","sessionid":"hpb-own","resumeid":"opaque-resume-secret","userid":"fixture-user"}})).await;
}
async fn join(socket: &mut ServerSocket) {
    let request = receive(socket).await;
    assert_eq!(request["type"], "room");
    assert_eq!(request["room"]["roomid"], "testroom");
    assert_eq!(request["room"]["sessionid"], "talk-own");
    send(
        socket,
        json!({"id":request["id"],"type":"room","room":{"roomid":"testroom"}}),
    )
    .await;
    send(
        socket,
        json!({"type":"event","event":{"target":"room","type":"join","join":[
            {"sessionid":"hpb-other","roomsessionid":"talk-other"}
        ]}}),
    )
    .await;
}
async fn wait_for(shared: &Shared, predicate: impl Fn(&State) -> bool) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if predicate(&shared.state.lock().unwrap()) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[test]
fn opaque_settings_preserve_supplied_auth_and_reject_unsafe_routes() {
    let s = settings("https://hpb.invalid/signal/");
    assert_eq!(s.servers[0].as_str(), "wss://hpb.invalid/signal/spreed");
    assert_eq!(
        s.hello("1", "https://account.invalid/backend", false)
            .unwrap()["hello"]["auth"]["params"]["ticket"],
        "opaque-v1-ticket"
    );
    for server in [
        "https://name:password@hpb.invalid",
        "https://hpb.invalid/a/../b",
        "https://hpb.invalid/?auth=token",
        "https://hpb.invalid/#token",
        "http://hpb.invalid",
        "https://hpb.invalid./",
    ] {
        assert!(
            ExternalSettings::decode(
                &json!({"server":server,"userId":"fixture-user","ticket":"opaque"})
            )
            .is_err()
        );
    }
    assert!(matches!(
        ExternalSettings::decode(
            &json!({"server":"https://hpb.invalid","userId":"fixture-user","helloAuthParams":{"1.0":{"userid":"other","ticket":"opaque"}}})
        ),
        Err(ErrorKind::WrongScope)
    ));
    assert!(matches!(
        ExternalSettings::decode(
            &json!({"server":"https://hpb.invalid","userId":"fixture-user","ticket":"opaque","federation":{"server":"https://other.invalid"}})
        ),
        Err(ErrorKind::Unsupported)
    ));
    let model = SignalingSettings::decode(&json!({"signalingMode":"external","server":"https://hpb.invalid","userId":"fixture-user","ticket":"redacted-ticket","stunservers":[],"turnservers":[]})).unwrap();
    let debug = format!("{model:?}");
    assert!(
        !debug.contains("redacted-ticket")
            && !debug.contains("fixture-user")
            && !debug.contains("hpb.invalid")
    );
}

#[test]
fn invalidations_coalesce_overload_into_full_reconciliation() {
    let mut mailbox = Mailbox::default();
    for n in 0..33 {
        mailbox.push(LiveEvent::ChatAvailable {
            room: RoomToken::parse(&format!("room{n}")).unwrap(),
        });
    }
    assert_eq!(mailbox.drain(), vec![LiveEvent::RefreshAll]);
    mailbox.push(LiveEvent::CallChanged { room: room() });
    mailbox.push(LiveEvent::CallChanged { room: room() });
    assert_eq!(mailbox.drain().len(), 1);
    assert_eq!(
        wire::hint(
            &json!({"target":"roomlist","type":"update","update":{"roomid":"testroom"}}),
            None
        )
        .unwrap(),
        Some(LiveEvent::RoomChanged { room: room() })
    );
}

#[tokio::test]
async fn typed_drain_is_scoped_and_internal_mode_is_explicit() {
    let server = crate::tests::Server::start(vec![crate::tests::reply(
        json!({"signalingMode":"internal","stunservers":[],"turnservers":[]}),
    )])
    .await;
    let (mut client, runtime) = crate::tests::client(&server);
    let result = client
        .poll_live_events(&crate::tests::context(&runtime))
        .await
        .unwrap();
    assert_eq!(result.scope, crate::tests::scope());
    assert_eq!(result.state, LiveState::Internal);
    assert!(result.events.is_empty());
    let mut wrong = crate::tests::context(&runtime);
    wrong.scope.runtime = LeaseId::from_key(999);
    assert!(
        matches!(client.poll_live_events(&wrong).await, Err(e) if e.kind == ErrorKind::WrongScope)
    );
    assert_eq!(server.count(), 1);
}

#[test]
fn only_current_correlated_subscription_denial_is_recoverable() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    {
        let mut state = shared.state.lock().unwrap();
        let room = state.room.as_mut().unwrap();
        room.subscription_requests = vec!["old".into(), "current".into()];
        room.selected.push(Selection {
            request_id: Some("current".into()),
            stream: StreamKind::Screen,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: None,
            publisher: false,
        });
    }
    let denied = |id: &str| json!({"id":id,"type":"error","error":{"code":"not_allowed"}});
    shared.incoming(&denied("old")).unwrap();
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .signals
            .is_empty()
    );
    shared.incoming(&denied("current")).unwrap();
    shared.incoming(&denied("current")).unwrap();
    {
        let state = shared.state.lock().unwrap();
        let room = state.room.as_ref().unwrap();
        assert_eq!(room.signals.len(), 1);
        assert!(
            matches!(&room.signals[0].1, Event::SubscriptionRejected { from, stream: StreamKind::Screen } if from == &peer("talk-other"))
        );
        assert!(!room.cancellation.is_cancelled());
    }
    assert_eq!(
        shared.incoming(&denied("unrelated")),
        Err(ErrorKind::PermissionDenied)
    );
    assert_eq!(
        shared.incoming(&json!({"id":"current","type":"error","error":{"code":"auth_failed"}})),
        Err(ErrorKind::PermissionDenied)
    );
}

#[test]
fn room_mapping_waits_for_both_hpb_call_sessions_and_handles_event_order() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    let update = |users: Value| json!({"type":"event","event":{"target":"participants","type":"update","update":{"roomid":"testroom","users":users}}});
    shared
        .incoming(&update(json!([{"sessionId":"hpb-other","inCall":7}])))
        .unwrap();
    shared.incoming(&json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":"talk-other"}]}})).unwrap();
    {
        let state = shared.state.lock().unwrap();
        assert!(
            state
                .room
                .as_ref()
                .unwrap()
                .signals
                .iter()
                .all(|(_, e)| !matches!(e, Event::SfuMembership(m) if !m.is_empty()))
        );
    }
    shared
        .incoming(&update(
            json!([{"sessionId":"hpb-own","inCall":true},{"sessionId":"hpb-other","inCall":7}]),
        ))
        .unwrap();
    {
        let mut state = shared.state.lock().unwrap();
        let room = state.room.as_mut().unwrap();
        assert!(
            room.signals
                .iter()
                .any(|(_, e)| matches!(e, Event::SfuMembership(m) if m == &[peer("talk-other")]))
        );
        assert!(room.selected.is_empty()); // Still no REST admission.
        room.signals.clear();
        room.signal_bytes = 0;
    }
    shared
        .incoming(&update(
            json!([{"sessionId":"hpb-own","inCall":1},{"sessionId":"hpb-other","inCall":0}]),
        ))
        .unwrap();
    let state = shared.state.lock().unwrap();
    assert!(
        state
            .room
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .any(|(_, e)| matches!(e, Event::SfuMembership(m) if m.is_empty()))
    );
}

#[test]
fn mapping_never_selects_media_and_sfu_supplied_sid_is_required() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    shared.incoming(&json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":"talk-other"}]}})).unwrap();
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .selected
            .is_empty()
    );
    let offer = |sid: &str| json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"offer","roomType":"video","sid":sid,"payload":{"type":"offer","sdp":"v=0\r\n"}}}});
    shared.incoming(&offer("unsolicited")).unwrap();
    {
        let mut state = shared.state.lock().unwrap();
        let room = state.room.as_mut().unwrap();
        assert!(
            !room
                .signals
                .iter()
                .any(|(_, e)| matches!(e, Event::Signal { .. }))
        );
        room.selected.push(Selection {
            stream: StreamKind::Camera,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: None,
            publisher: false,
            request_id: None,
        });
    }
    shared.incoming(&offer("sfu-created-sid")).unwrap();
    shared.incoming(&offer("obsolete-sid")).unwrap();
    let state = shared.state.lock().unwrap();
    let room = state.room.as_ref().unwrap();
    assert_eq!(
        room.signals
            .iter()
            .filter(|(_, e)| matches!(e, Event::Signal { .. }))
            .count(),
        1
    );
    assert!(
        room.selected[0].sid.as_ref().unwrap() == &Negotiation::parse("sfu-created-sid").unwrap()
    );
}

#[test]
fn mapped_identity_cannot_change_and_media_queue_overflow_closes_route() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    let message = |talk: &str| json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":talk}]}});
    shared.incoming(&message("talk-other")).unwrap();
    assert_eq!(
        shared.incoming(&message("talk-impostor")),
        Err(ErrorKind::WrongScope)
    );
    let mut state = shared.state.lock().unwrap();
    let room = state.room.as_mut().unwrap();
    for _ in 0..83 {
        room.push(Event::PolicyChanged).unwrap();
    }
    assert_eq!(room.push(Event::PolicyChanged), Err(ErrorKind::Busy));
    assert!(room.cancellation.is_cancelled());
}

#[tokio::test]
async fn hello_token_validity_failure_is_scoped_to_signaling_admission() {
    for code in ["token_expired", "token_not_valid_yet"] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut socket = accept(&listener).await;
            send(
                &mut socket,
                json!({"type":"welcome","welcome":{"features":["hello-v2"]}}),
            )
            .await;
            let hello = receive(&mut socket).await;
            send(
                &mut socket,
                json!({"id":hello["id"],"type":"error","error":{"code":code}}),
            )
            .await;
        });
        let (client, binding, runtime) = account();
        let owner = Owner::start(binding, settings(&endpoint)).unwrap();
        let handle = owner.handle();
        let context = context(&runtime, &handle.shared);
        let result = handle.join(&context, room(), peer("talk-own")).await;
        assert!(matches!(result, Err(ErrorKind::PermissionDenied)));
        assert!(!client.cancellation().is_cancelled());
        assert!(client.validate(&context).is_ok());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn authenticated_session_routes_subscriber_answer_and_confirms_retirement() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        join(&mut socket).await;
        let request = receive(&mut socket).await;
        assert_eq!(request["message"]["recipient"]["sessionid"], "hpb-other");
        assert_eq!(request["message"]["data"]["type"], "requestoffer");
        assert!(request["message"]["data"].get("sid").is_none());
        send(&mut socket, json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"offer","roomType":"video","sid":"sfu-created-sid","payload":{"type":"offer","sdp":"v=0\r\n"}}}})).await;
        let answer = receive(&mut socket).await;
        assert_eq!(answer["message"]["recipient"]["sessionid"], "hpb-other");
        assert_eq!(answer["message"]["data"]["sid"], "sfu-created-sid");
        assert_eq!(answer["message"]["data"]["type"], "answer");
        let activity = receive(&mut socket).await;
        assert_eq!(activity["message"]["recipient"]["sessionid"], "hpb-other");
        assert_eq!(activity["message"]["data"]["type"], "raiseHand");
        assert_eq!(activity["message"]["data"]["payload"]["state"], true);
        assert!(activity["message"]["data"].get("sid").is_none());
        let force_mute = receive(&mut socket).await;
        assert_eq!(force_mute["message"]["recipient"]["sessionid"], "hpb-other");
        assert_eq!(
            force_mute["message"]["data"]["payload"]["action"],
            "forceMute"
        );
        assert_eq!(
            force_mute["message"]["data"]["payload"]["peerId"],
            "hpb-other"
        );
        for stream in ["video", "screen"] {
            let publication = receive(&mut socket).await;
            assert_eq!(
                publication["message"]["recipient"]["sessionid"],
                "hpb-other"
            );
            assert_eq!(publication["message"]["data"]["type"], "sendoffer");
            assert_eq!(publication["message"]["data"]["roomType"], stream);
            assert!(publication["message"]["data"].get("sid").is_none());
        }
        let bye = receive(&mut socket).await;
        assert_eq!(bye["type"], "bye");
        send(&mut socket, json!({"id":bye["id"],"type":"bye","bye":{}})).await;
    });
    let (_client, binding, runtime) = account();
    let owner = Owner::start(binding, settings(&endpoint)).unwrap();
    let handle = owner.handle();
    let route = handle
        .join(&context(&runtime, &handle.shared), room(), peer("talk-own"))
        .await
        .unwrap();
    wait_for(&handle.shared, |s| {
        s.room.as_ref().unwrap().members.len() == 1
    })
    .await;
    route
        .request_offer(&context(&runtime, &handle.shared), &peer("talk-other"))
        .await
        .unwrap();
    wait_for(&handle.shared, |s| {
        s.room.as_ref().unwrap().selected[0].sid.is_some()
    })
    .await;
    let sid = Negotiation::parse("sfu-created-sid").unwrap();
    route.select(peer("talk-other"), sid.clone()).unwrap();
    assert_eq!(
        route.select(peer("talk-other"), Negotiation::parse("wrong-sid").unwrap()),
        Err(ErrorKind::Conflict)
    );
    route
        .send(
            &context(&runtime, &handle.shared),
            &peer("talk-other"),
            Some(&sid),
            Some(Signal::Description(
                crate::signaling::SessionDescription::new(
                    crate::signaling::DescriptionKind::Answer,
                    "v=0\r\n",
                )
                .unwrap(),
            )),
        )
        .await
        .unwrap();
    route
        .send_activity(
            &context(&runtime, &handle.shared),
            &peer("talk-other"),
            crate::signaling::activity::Activity::RaisedHand {
                state: true,
                timestamp: 42,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        route
            .send_activity(
                &context(&runtime, &handle.shared),
                &peer("unknown"),
                crate::signaling::activity::Activity::Reaction("👍".into()),
            )
            .await,
        Err(ErrorKind::WrongScope)
    );
    route
        .send_force_mute(
            &context(&runtime, &handle.shared),
            &peer("talk-other"),
            &peer("talk-other"),
        )
        .await
        .unwrap();
    for stream in [StreamKind::Camera, StreamKind::Screen] {
        route
            .send_stream(
                &context(&runtime, &handle.shared),
                &peer("talk-other"),
                None,
                Some(Signal::SendOffer),
                stream,
            )
            .await
            .unwrap();
    }
    // Rejected media mutations retire the route, so test membership rejection
    // only after both valid publications have reached the socket.
    assert_eq!(
        route
            .send_stream(
                &context(&runtime, &handle.shared),
                &peer("unknown"),
                None,
                Some(Signal::SendOffer),
                StreamKind::Camera,
            )
            .await,
        Err(ErrorKind::WrongScope)
    );
    route.close();
    assert!(route.is_closed());
    server.await.unwrap();
    wait_for(&handle.shared, |s| s.room.is_none()).await;
    assert!(!owner.retirement_pending());
    assert_eq!(
        route
            .send_stream(
                &context(&runtime, &handle.shared),
                &peer("talk-other"),
                None,
                Some(Signal::SendOffer),
                StreamKind::Camera,
            )
            .await,
        Err(ErrorKind::Cancelled)
    );
    assert!(matches!(
        route.poll(&context(&runtime, &handle.shared)).await,
        Err(ErrorKind::Cancelled)
    ));
}

#[tokio::test]
async fn resume_preserves_current_route_but_lost_session_never_rejoins() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
    let (disconnect, disconnected) = oneshot::channel();
    let (lose, lost) = oneshot::channel();
    let (fresh, fresh_seen) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        join(&mut socket).await;
        disconnected.await.unwrap();
        drop(socket);
        let mut socket = accept(&listener).await;
        hello(&mut socket, true).await;
        lost.await.unwrap();
        drop(socket);
        let mut socket = accept(&listener).await;
        send(&mut socket, json!({"type":"welcome","welcome":{"features":["hello-v2","mcu","audio-video-permissions","incall-all"]}})).await;
        let resume = receive(&mut socket).await;
        assert_eq!(resume["hello"]["resumeid"], "opaque-resume-secret");
        send(
            &mut socket,
            json!({"id":resume["id"],"type":"error","error":{"code":"no_such_session"}}),
        )
        .await;
        let new = receive(&mut socket).await;
        assert!(new["hello"].get("resumeid").is_none());
        send(&mut socket, json!({"id":new["id"],"type":"hello","hello":{"version":"2.0","sessionid":"hpb-new","resumeid":"new-secret","userid":"fixture-user"}})).await;
        let _ = fresh.send(());
        // A fresh account session sends neither a room join nor media replay.
        assert!(
            tokio::time::timeout(Duration::from_millis(100), socket.next())
                .await
                .is_err()
        );
    });
    let (_client, binding, runtime) = account();
    let owner = Owner::start(binding, settings(&endpoint)).unwrap();
    let handle = owner.handle();
    let route = handle
        .join(&context(&runtime, &handle.shared), room(), peer("talk-own"))
        .await
        .unwrap();
    disconnect.send(()).unwrap();
    wait_for(&handle.shared, |s| {
        s.room.as_ref().is_some_and(|r| {
            r.signals
                .iter()
                .any(|(_, e)| matches!(e, Event::TransportResumed))
        })
    })
    .await;
    assert!(!route.is_closed());
    let batch = route
        .poll(&context(&runtime, &handle.shared))
        .await
        .unwrap();
    assert!(
        batch
            .into_events()
            .iter()
            .any(|e| matches!(e, Event::TransportInterrupted))
    );
    lose.send(()).unwrap();
    fresh_seen.await.unwrap();
    assert!(route.is_closed());
    assert!(route.session_replaced());
    assert!(handle.shared.state.lock().unwrap().room.is_none());
    server.await.unwrap();
}

#[tokio::test]
async fn wrong_scope_cancelled_or_expired_actions_do_not_enter_command_queue() {
    let (_client, binding, runtime) = account();
    let shared = Arc::new(state(binding));
    let (commands, mut queued) = mpsc::channel(1);
    let handle = Handle {
        shared: shared.clone(),
        commands,
    };
    let route = Route {
        replaced: Arc::new(AtomicBool::new(false)),
        handle,
        generation: 1,
        cancellation: shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .cancellation
            .clone(),
    };
    let mut wrong = context(&runtime, &shared);
    wrong.scope.runtime = LeaseId::from_key(999);
    assert_eq!(
        route.send(&wrong, &peer("talk-own"), None, None).await,
        Err(ErrorKind::WrongScope)
    );
    let cancelled = context(&runtime, &shared);
    cancelled.cancellation.cancel();
    assert_eq!(
        route.send(&cancelled, &peer("talk-own"), None, None).await,
        Err(ErrorKind::Cancelled)
    );
    let mut expired = context(&runtime, &shared);
    expired.deadline.millis = 0;
    assert_eq!(
        route.send(&expired, &peer("talk-own"), None, None).await,
        Err(ErrorKind::Deadline)
    );
    assert!(queued.try_recv().is_err());
    shared.account.cancellation.cancel();
    assert!(route.is_closed());
}

/// Invoked only by the finite disposable TLS fixture. The process receives its
/// native CA file at startup; the connector itself has no trust override.
#[cfg(feature = "backend-test")]
#[tokio::test]
#[ignore = "requires disposable WSS servers and process native trust fixture"]
async fn real_wss_native_roots() {
    let path = std::env::var_os("NMT_WSS_FIXTURE").expect("WSS fixture path");
    let bytes = std::fs::read(path).expect("WSS fixture");
    assert!(bytes.len() <= 16 * 1024);
    let endpoints: Value = serde_json::from_slice(&bytes).unwrap();
    for name in ["trusted", "untrusted", "wrong_host"] {
        let endpoint = endpoints[name].as_str().expect("WSS endpoint");
        let url = url::Url::parse(endpoint).unwrap();
        assert_eq!(url.scheme(), "wss");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        let (_client, binding, _runtime) = account();
        let shared = state(binding);
        *shared.settings.lock().unwrap() = settings(endpoint);
        let result = session::connect(&shared, 0).await;
        if name == "trusted" {
            let mut socket = result.expect("trusted native-root WSS upgrade");
            socket.close(None).await.unwrap();
        } else {
            assert_eq!(result.err(), Some(ErrorKind::Transport), "{name}");
        }
    }
}

#[test]
fn chat_typing_is_scoped_to_current_signaling_membership_and_expires() {
    let (_client, binding, _runtime) = account();
    let shared = state(binding);
    {
        let mut guard = shared.state.lock().unwrap();
        let room = guard.room.as_mut().unwrap();
        room.purpose = RoomPurpose::Chat;
        room.members.push(wire::Member {
            sfu: peer("hpb-other"),
            talk: peer("talk-other"),
        });
    }
    let frame = |sender: &str, kind: &str| {
        json!({"type":"message","message":{
            "sender":{"type":"session","sessionid":sender},
            "data":{"type":kind,"to":"hpb-own"}
        }})
    };
    shared
        .incoming(&frame("hpb-unknown", "startedTyping"))
        .unwrap();
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .typing
            .is_empty()
    );
    shared
        .incoming(&frame("hpb-other", "startedTyping"))
        .unwrap();
    {
        let mut guard = shared.state.lock().unwrap();
        let room = guard.room.as_mut().unwrap();
        assert_eq!(room.typing.len(), 1);
        assert!(room.typing[0].0.matches(&peer("talk-other")));
        room.typing[0].1 = std::time::Instant::now() - Duration::from_secs(9);
    }
    let route = Route {
        replaced: Arc::new(AtomicBool::new(false)),
        handle: Handle {
            shared: Arc::new(shared),
            commands: mpsc::channel(1).0,
        },
        generation: 1,
        cancellation: Cancellation::default(),
    };
    assert!(route.typing_sessions().unwrap().is_empty());
    route
        .handle
        .shared
        .incoming(&frame("hpb-other", "startedTyping"))
        .unwrap();
    route
        .handle
        .shared
        .incoming(&frame("hpb-other", "stoppedTyping"))
        .unwrap();
    assert!(route.typing_sessions().unwrap().is_empty());
    route
        .handle
        .shared
        .incoming(&frame("hpb-other", "startedTyping"))
        .unwrap();
    route
        .handle
        .shared
        .incoming(
            &json!({"type":"event","event":{"target":"room","type":"leave","leave":["hpb-other"]}}),
        )
        .unwrap();
    assert!(route.typing_sessions().unwrap().is_empty());
    assert!(
        route
            .handle
            .shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .signals
            .is_empty()
    );
}

#[test]
fn chat_typing_wire_validates_sender_and_targets_one_session() {
    let msg = wire::typing_message(&peer("hpb-peer"), true);
    assert_eq!(msg["message"]["recipient"]["sessionid"], "hpb-peer");
    assert_eq!(msg["message"]["data"]["type"], "startedTyping");
    let stopped = wire::typing_message(&peer("hpb-peer"), false);
    assert_eq!(stopped["message"]["data"]["type"], "stoppedTyping");
    let spoofed = json!({"sender":{"type":"session","sessionid":"hpb-peer"},"data":{"from":"hpb-other","type":"startedTyping"}});
    assert!(matches!(wire::typing(&spoofed), Err(ErrorKind::WrongScope)));
}
#[test]
fn screen_offer_and_stop_cannot_replace_camera_or_cross_membership() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    let offer = |sender: &str, kind: &str, sid: &str| json!({"type":"message","message":{"sender":{"type":"session","sessionid":sender},"data":{"type":"offer","roomType":kind,"sid":sid,"payload":{"type":"offer","sdp":"v=0\r\n"}}}});
    shared
        .incoming(&offer("unknown", "screen", "rogue"))
        .unwrap();
    shared.incoming(&json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":"talk-other"}]}})).unwrap();
    {
        let mut state = shared.state.lock().unwrap();
        state.room.as_mut().unwrap().selected.push(Selection {
            stream: StreamKind::Camera,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: Some(Negotiation::parse("camera").unwrap()),
            publisher: false,
            request_id: None,
        });
    }
    shared
        .incoming(&offer("hpb-other", "screen", "screen"))
        .unwrap();
    shared.incoming(&json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"unshareScreen","roomType":"screen"}}})).unwrap();
    let state = shared.state.lock().unwrap();
    let room = state.room.as_ref().unwrap();
    assert_eq!(room.selected.len(), 2);
    assert_eq!(room.selected[0].stream, StreamKind::Camera);
    assert!(
        room.selected[0]
            .sid
            .as_ref()
            .unwrap()
            .matches(&Negotiation::parse("camera").unwrap())
    );
    assert!(room.signals.iter().any(|(_, e)| matches!(
        e,
        Event::Signal {
            stream: StreamKind::Screen,
            ..
        }
    )));
    assert!(
        room.signals
            .iter()
            .any(|(_, e)| matches!(e, Event::ScreenStopped { .. }))
    );
    assert!(!room.cancellation.is_cancelled());
}

#[tokio::test]
async fn incomplete_upgrade_during_restart_retries_transport_without_room_replay() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
    let (finish, finished) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream); // EOF before the websocket upgrade response is complete.
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        finished.await.unwrap();
    });
    let (_client, binding, _) = account();
    let owner = Owner::start(binding, settings(&endpoint)).unwrap();
    let handle = owner.handle();
    wait_for(&handle.shared, |s| s.status == LiveState::Connected).await;
    {
        let state = handle.shared.state.lock().unwrap();
        assert_eq!(state.first_transport_failure, Some(ErrorKind::Transport));
        assert!(state.room.is_none());
    }
    finish.send(()).unwrap();
    server.await.unwrap();
}

#[test]
fn obsolete_subscriber_offer_cannot_bind_the_replacement_generation() {
    let (_client, binding, _) = account();
    let shared = Arc::new(state(binding));
    let (commands, _) = mpsc::channel(1);
    let cancellation = shared
        .state
        .lock()
        .unwrap()
        .room
        .as_ref()
        .unwrap()
        .cancellation
        .clone();
    let route = Route {
        replaced: Arc::new(AtomicBool::new(false)),
        handle: Handle {
            shared: shared.clone(),
            commands,
        },
        generation: 1,
        cancellation,
    };
    {
        let mut state = shared.state.lock().unwrap();
        let room = state.room.as_mut().unwrap();
        room.selected.push(Selection {
            stream: StreamKind::Camera,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: Some(Negotiation::parse("old").unwrap()),
            publisher: false,
            request_id: None,
        });
    }
    route
        .reset_subscription(&peer("talk-other"), StreamKind::Camera)
        .unwrap();
    {
        let mut state = shared.state.lock().unwrap();
        state.room.as_mut().unwrap().selected.push(Selection {
            stream: StreamKind::Camera,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: None,
            publisher: false,
            request_id: None,
        });
    }
    let offer = |sid: &str| json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"offer","roomType":"video","sid":sid,"payload":{"type":"offer","sdp":"v=0\r\n"}}}});
    shared.incoming(&offer("old")).unwrap();
    assert!(
        shared.state.lock().unwrap().room.as_ref().unwrap().selected[0]
            .sid
            .is_none()
    );
    shared.incoming(&offer("new")).unwrap();
    assert!(
        shared.state.lock().unwrap().room.as_ref().unwrap().selected[0]
            .sid
            .as_ref()
            .is_some_and(|sid| sid.matches(&Negotiation::parse("new").unwrap()))
    );
}

#[tokio::test]
async fn stopped_camera_subscription_can_readmit_a_current_member_without_old_sid() {
    let (_client, binding, runtime) = account();
    let shared = Arc::new(state(binding));
    let (commands, mut receive) = mpsc::channel(1);
    let cancellation = shared
        .state
        .lock()
        .unwrap()
        .room
        .as_ref()
        .unwrap()
        .cancellation
        .clone();
    let route = Route {
        replaced: Arc::new(AtomicBool::new(false)),
        handle: Handle {
            shared: shared.clone(),
            commands,
        },
        generation: 1,
        cancellation,
    };
    {
        let mut state = shared.state.lock().unwrap();
        let room = state.room.as_mut().unwrap();
        room.members.push(wire::Member {
            sfu: peer("hpb-other"),
            talk: peer("talk-other"),
        });
        room.selected.push(Selection {
            stream: StreamKind::Camera,
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: Some(Negotiation::parse("old").unwrap()),
            publisher: false,
            request_id: None,
        });
    }
    route.remove_stream(&peer("talk-other"), StreamKind::Camera);
    let writer = tokio::spawn(async move {
        let Command::Send {
            sid, signal, reply, ..
        } = receive.recv().await.unwrap()
        else {
            panic!("subscription command");
        };
        assert!(sid.is_none() && signal.is_none());
        reply.send(Ok(())).unwrap();
    });
    route
        .request_offer(&context(&runtime, &shared), &peer("talk-other"))
        .await
        .unwrap();
    writer.await.unwrap();
    let offer = |sid: &str| json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"offer","roomType":"video","sid":sid,"payload":{"type":"offer","sdp":"v=0\r\n"}}}});
    shared.incoming(&offer("old")).unwrap();
    assert!(
        shared.state.lock().unwrap().room.as_ref().unwrap().selected[0]
            .sid
            .is_none()
    );
    shared.incoming(&offer("fresh")).unwrap();
    assert!(
        shared.state.lock().unwrap().room.as_ref().unwrap().selected[0]
            .sid
            .as_ref()
            .unwrap()
            .matches(&Negotiation::parse("fresh").unwrap())
    );
}
#[test]
fn call_encryption_needs_a_mapped_sender_carries_both_session_ids_and_never_ends_the_call() {
    let (_client, binding, _) = account();
    let shared = state(binding);
    let message = |sender: &str, payload: serde_json::Value| {
        json!({"type":"message","message":{
            "sender":{"type":"session","sessionid":sender},
            "data":{"type":"message","to":"hpb-own","payload":payload}}})
    };
    let start = json!({"id":"1","type":"encryption.start","identity":"a","key":"b"});
    // Unmapped sessions are ignored.
    shared
        .incoming(&message("hpb-other", start.clone()))
        .unwrap();
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .signals
            .is_empty()
    );
    shared.incoming(&json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":"talk-other"}]}})).unwrap();
    shared
        .incoming(&message("hpb-other", start.clone()))
        .unwrap();
    // Not key exchange: no event.
    shared
        .incoming(&message("hpb-other", json!({"type":"other"})))
        .unwrap();
    {
        let locked = shared.state.lock().unwrap();
        let signals: Vec<_> = locked
            .room
            .as_ref()
            .unwrap()
            .signals
            .iter()
            .filter(|(_, event)| matches!(event, Event::Encryption { .. }))
            .collect();
        assert_eq!(signals.len(), 1);
        assert!(
            matches!(&signals[0].1, Event::Encryption { from, wire, payload }
            if from == &peer("talk-other") && wire == &peer("hpb-other") && *payload == start)
        );
    }
    // An oversized payload is refused with a protocol error; a flood is dropped,
    // not turned into a cancelled call.
    assert!(
        shared
            .incoming(&message(
                "hpb-other",
                json!({"type":"encryption.setkey","pad":"x".repeat(9000)})
            ))
            .is_err()
    );
    for _ in 0..100 {
        shared
            .incoming(&message("hpb-other", start.clone()))
            .unwrap();
    }
    let locked = shared.state.lock().unwrap();
    let room = locked.room.as_ref().unwrap();
    assert!(!room.cancellation.is_cancelled());
    let encryptions = room
        .signals
        .iter()
        .filter(|(_, event)| matches!(event, Event::Encryption { .. }))
        .count();
    assert!(encryptions <= 32);
}
#[test]
fn call_activity_requires_a_mapped_sender_and_never_crowds_out_policy() {
    use crate::signaling::activity::Activity;
    let (_client, binding, _) = account();
    let shared = state(binding);
    let message = |sender: &str, state: bool, timestamp: u64| {
        json!({
            "type":"message", "message": {
                "sender":{"type":"session", "sessionid":sender},
                "data":{"type":"raiseHand", "roomType":"video", "payload":{"state":state,"timestamp":timestamp}}
            }
        })
    };
    shared.incoming(&message("hpb-other", true, 1)).unwrap();
    assert!(
        shared
            .state
            .lock()
            .unwrap()
            .room
            .as_ref()
            .unwrap()
            .signals
            .is_empty()
    );
    shared.incoming(&json!({"type":"event","event":{"target":"room","type":"join","join":[{"sessionid":"hpb-other","roomsessionid":"talk-other"}]}})).unwrap();
    shared.incoming(&json!({"type":"message","message":{"sender":{"type":"session","sessionid":"hpb-other"},"data":{"type":"control","roomType":"video","payload":{"action":"forceMute","peerId":"hpb-own"}}}})).unwrap();
    shared.incoming(&message("hpb-other", true, 1)).unwrap();
    shared.incoming(&message("hpb-other", false, 2)).unwrap();
    shared.incoming(&message("hpb-other", true, 1)).unwrap();
    let mut locked = shared.state.lock().unwrap();
    let room = locked.room.as_mut().unwrap();
    assert!(
        room.signals
            .iter()
            .any(|(_, event)| matches!(event, Event::Activity {
        from, activity: Activity::RaisedHand { state: false, timestamp: 2 }
    } if from == &peer("talk-other")))
    );
    assert!(room.signals.iter().any(
        |(_, event)| matches!(event, Event::ForceMuteBy { from } if from == &peer("talk-other"))
    ));
    // Fill the queue with transient activity, then retain a mandatory update.
    for i in 0..20 {
        room.push_activity(peer(&format!("peer-{i}")), Activity::Reaction("👍".into()));
    }
    for _ in 0..82 {
        room.push(Event::PolicyChanged).unwrap();
    }
    assert!(!room.cancellation.is_cancelled());
    assert!(
        room.signals
            .iter()
            .any(|(_, event)| matches!(event, Event::PolicyChanged))
    );
}

#[tokio::test]
async fn network_resume_hint_replaces_a_live_socket_and_resumes_without_a_second_join() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/signal", listener.local_addr().unwrap());
    let (resumed, seen) = oneshot::channel();
    let (stop, stopped) = oneshot::channel();
    let server = tokio::spawn(async move {
        let mut socket = accept(&listener).await;
        hello(&mut socket, false).await;
        join(&mut socket).await;
        // The peer stays healthy: only the client's connectivity epoch changes.
        let closed = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap();
        assert!(closed.is_none() || matches!(closed, Some(Err(_)) | Some(Ok(Message::Close(_)))));
        let mut socket = accept(&listener).await;
        hello(&mut socket, true).await;
        resumed.send(()).unwrap();
        tokio::select! { _=stopped=>(), value=socket.next()=>panic!("resume sent an unsolicited command: {}",value.is_some()) }
    });
    let (_client, binding, runtime) = account();
    let owner = Owner::start(binding, settings(&endpoint)).unwrap();
    let handle = owner.handle();
    let route = handle
        .join(&context(&runtime, &handle.shared), room(), peer("talk-own"))
        .await
        .unwrap();
    let start = std::time::Instant::now();
    runtime.network_changed();
    tokio::time::timeout(Duration::from_secs(3), seen)
        .await
        .unwrap()
        .unwrap();
    wait_for(&handle.shared, |state| {
        state.room.as_ref().is_some_and(|room| {
            room.signals
                .iter()
                .any(|(_, event)| matches!(event, Event::TransportResumed))
        })
    })
    .await;
    assert!(start.elapsed() < Duration::from_secs(3));
    assert!(!route.is_closed());
    let batch = route
        .poll(&context(&runtime, &handle.shared))
        .await
        .unwrap();
    let events = batch.into_events();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::TransportInterrupted))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::TransportResumed))
            .count(),
        1
    );
    stop.send(()).unwrap();
    server.await.unwrap();
}
