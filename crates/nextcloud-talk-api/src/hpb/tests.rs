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
        state: Mutex::new(State {
            status: LiveState::Connected,
            first_transport_failure: None,
            events: Mailbox::default(),
            session: Some(peer("hpb-own")),
            mcu: true,
            generation: 1,
            room: Some(Room {
                token: room(),
                own: peer("talk-own"),
                generation: 1,
                cancellation: Cancellation::default(),
                joined: true,
                members: Vec::new(),
                selected: Vec::new(),
                retired: Vec::new(),
                signals: VecDeque::new(),
                signal_bytes: 0,
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
            talk: peer("talk-other"),
            sfu: peer("hpb-other"),
            sid: None,
            publisher: false,
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
    route.close();
    assert!(route.is_closed());
    server.await.unwrap();
    wait_for(&handle.shared, |s| s.room.is_none()).await;
    assert!(!owner.retirement_pending());
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
