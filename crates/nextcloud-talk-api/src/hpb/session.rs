use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::time::Instant;
use tokio::net::TcpStream;
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream, connect_async_tls_with_config,
    tungstenite::{self, Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
#[derive(Default)]
struct RetryBudget {
    failures: usize,
}
impl RetryBudget {
    fn interrupted(&mut self) -> bool {
        if self.failures >= 4 {
            return false;
        }
        self.failures += 1;
        true
    }
    fn connected_for(&mut self, duration: Duration) {
        // The account socket can live for days. Bound one unstable episode,
        // rather than exhausting rejoin forever after four unrelated outages.
        // A handshake alone cannot renew the budget during a rapid flap.
        if duration >= Duration::from_secs(30) {
            self.failures = 0;
        }
    }
}
struct PendingJoin {
    id: String,
    generation: u64,
    deadline: Moment,
    cancel: Cancellation,
    reply: oneshot::Sender<Result<(), ErrorKind>>,
}
#[derive(Clone)]
pub(super) struct Resume {
    pub(super) session: PeerSession,
    pub(super) secret: Zeroizing<String>,
}
fn deadline(shared: &Shared, millis: u64) -> Moment {
    let now = shared.account.runtime.now();
    Moment {
        epoch: now.epoch,
        millis: now.millis.saturating_add(millis),
    }
}
fn next_id(counter: &mut u64) -> Result<String, ErrorKind> {
    *counter = counter.checked_add(1).ok_or(ErrorKind::Busy)?;
    Ok(counter.to_string())
}
fn socket_error(error: tungstenite::Error) -> ErrorKind {
    match error {
        tungstenite::Error::Http(response) if response.status().is_redirection() => {
            ErrorKind::RedirectRejected
        }
        tungstenite::Error::Capacity(_) => ErrorKind::TooLarge,
        // An abrupt TCP/TLS EOF is transport loss. Other protocol violations
        // still terminate the session and never enter the resume path.
        tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
            | tungstenite::error::ProtocolError::HandshakeIncomplete,
        ) => ErrorKind::Transport,
        tungstenite::Error::Protocol(_) | tungstenite::Error::Utf8(_) => ErrorKind::Malformed,
        _ => ErrorKind::Transport,
    }
}
async fn write(socket: &mut Socket, value: Value) -> Result<(), ErrorKind> {
    let bytes = serde_json::to_string(&value).map_err(|_| ErrorKind::Malformed)?;
    if bytes.len() > wire::FRAME_BYTES {
        return Err(ErrorKind::TooLarge);
    }
    socket
        .send(Message::Text(bytes.into()))
        .await
        .map_err(socket_error)
}
async fn read(socket: &mut Socket) -> Result<Value, ErrorKind> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                if text.len() > wire::FRAME_BYTES {
                    return Err(ErrorKind::TooLarge);
                }
                return crate::json::json(text.as_bytes());
            }
            Some(Ok(Message::Ping(_))) => {
                socket.flush().await.map_err(socket_error)?;
            }
            Some(Ok(Message::Pong(_))) => {}
            Some(Ok(Message::Close(_))) | None => return Err(ErrorKind::Transport),
            Some(Ok(_)) => return Err(ErrorKind::Malformed),
            Some(Err(error)) => return Err(socket_error(error)),
        }
    }
}
pub(super) async fn connect(shared: &Shared, index: usize) -> Result<Socket, ErrorKind> {
    let url = {
        let settings = shared.settings.lock().map_err(|_| ErrorKind::Uncertain)?;
        settings
            .servers
            .get(index % settings.servers.len())
            .ok_or(ErrorKind::Malformed)?
            .clone()
    };
    // Build a fresh request, never from reqwest, its cookie jar or auth headers.
    // The Origin is the admitted Nextcloud origin, including a nondefault port.
    let mut request = url.as_str().into_client_request().map_err(socket_error)?;
    request.headers_mut().insert(
        "Origin",
        shared
            .account
            .origin
            .parse()
            .map_err(|_| ErrorKind::Malformed)?,
    );
    let mut config = WebSocketConfig::default();
    config.read_buffer_size = 16 * 1024;
    config.write_buffer_size = 0;
    config.max_write_buffer_size = 256 * 1024;
    config.max_message_size = Some(wire::FRAME_BYTES);
    config.max_frame_size = Some(wire::FRAME_BYTES);
    // reqwest enables aws-lc while the pinned media engine enables ring. Use
    // an explicit per-connection provider; ClientConfig::builder() cannot infer
    // a default from that combined graph. No process-global provider mutation.
    let connector = if url.scheme() == "wss" {
        let native = rustls_native_certs::load_native_certs();
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(native.certs);
        if roots.is_empty() {
            return Err(ErrorKind::Transport);
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| ErrorKind::Transport)?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Some(Connector::Rustls(Arc::new(config)))
    } else {
        None
    };
    // tokio-tungstenite performs one upgrade only: a 3xx fails instead of
    // forwarding the request or hello credentials to a redirected origin.
    until(shared, deadline(shared, 5000), &shared.stop, async {
        connect_async_tls_with_config(request, Some(config), false, connector)
            .await
            .map(|(socket, _)| socket)
            .map_err(socket_error)
    })
    .await
}
async fn authenticate(
    shared: &Shared,
    socket: &mut Socket,
    resume: &mut Option<Resume>,
    counter: &mut u64,
) -> Result<(), ErrorKind> {
    let mut features = match until(shared, deadline(shared, 3000), &shared.stop, read(socket)).await
    {
        Ok(v) if v["type"].as_str() == Some("welcome") => {
            wire::features(&v["welcome"]["features"])?
        }
        Ok(_) => return Err(ErrorKind::Malformed),
        Err(ErrorKind::Deadline) => Vec::new(), // Protocol v1 servers may omit welcome.
        Err(error) => return Err(error),
    };
    // At most one failed resume followed by one fresh hello. Losing the session
    // retires its room/call route; it never replays room joins or media commands.
    for _ in 0..2 {
        let id = next_id(counter)?;
        let resumed = resume.is_some();
        let hello = if let Some(previous) = resume.as_ref() {
            json!({"id":id,"type":"hello","hello":{"version":"1.0","resumeid":previous.secret.as_str()}})
        } else {
            shared
                .settings
                .lock()
                .map_err(|_| ErrorKind::Uncertain)?
                .hello(
                    &id,
                    &shared.account.backend,
                    features.iter().any(|s| s == "hello-v2"),
                )?
        };
        until(
            shared,
            deadline(shared, 3000),
            &shared.stop,
            write(socket, hello),
        )
        .await?;
        let value = until(shared, deadline(shared, 5000), &shared.stop, async {
            // A delayed welcome is allowed, but does not replace hello
            // correlation. No account event is accepted before authentication.
            for _ in 0..2 {
                let v = read(socket).await?;
                if v["type"].as_str() == Some("welcome") {
                    features = wire::features(&v["welcome"]["features"])?;
                    continue;
                }
                return Ok(v);
            }
            Err(ErrorKind::Malformed)
        })
        .await?;
        if value["id"].as_str() != Some(&id) {
            return Err(ErrorKind::WrongScope);
        }
        if value["type"].as_str() == Some("error") {
            if resumed && value["error"]["code"].as_str() == Some("no_such_session") {
                *resume = None;
                shared.invalidate_room(true);
                continue;
            }
            return Err(wire::failure(&value["error"]));
        }
        if value["type"].as_str() != Some("hello") {
            return Err(ErrorKind::Malformed);
        }
        let hello = &value["hello"];
        if !matches!(hello["version"].as_str(), Some("1.0" | "2.0")) {
            return Err(ErrorKind::Unsupported);
        }
        if (!resumed || hello.get("userid").is_some())
            && hello["userid"].as_str() != Some(shared.account.user.as_str())
        {
            return Err(ErrorKind::WrongScope);
        }
        let session = PeerSession::parse(wire::text(&hello["sessionid"], 512)?)?;
        if let Some(previous) = resume.as_ref()
            && previous.session != session
        {
            return Err(ErrorKind::WrongScope);
        }
        if let Some(v) = hello.get("resumeid") {
            *resume = Some(Resume {
                session: session.clone(),
                secret: Zeroizing::new(wire::text(v, 2048)?.into()),
            });
        } else if !resumed {
            return Err(ErrorKind::Malformed);
        }
        if features.is_empty() {
            features = wire::features(&hello["server"]["features"])?;
        }
        {
            let mut state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
            if state.session.as_ref().is_some_and(|old| old != &session) {
                state.first_transport_failure = None;
            }
            state.session = Some(session);
            state.mcu = ["mcu", "audio-video-permissions", "incall-all"]
                .iter()
                .all(|feature| features.iter().any(|s| s == feature));
        }
        shared.resumed()?;
        return Ok(());
    }
    Err(ErrorKind::PermissionDenied)
}
async fn retire(socket: &mut Socket, counter: &mut u64) -> Result<(), ErrorKind> {
    let id = next_id(counter)?;
    tokio::time::timeout(Duration::from_millis(750), async {
        write(socket, json!({"id":id,"type":"bye","bye":{}})).await?;
        // Only the matching bye proves server-side retirement. Pending room
        // messages can precede it; they are discarded and cannot revive media.
        for _ in 0..32 {
            let v = read(socket).await?;
            if v["type"].as_str() == Some("bye") && v["id"].as_str() == Some(&id) {
                return Ok(());
            }
        }
        Err(ErrorKind::Uncertain)
    })
    .await
    .map_err(|_| ErrorKind::Uncertain)?
}
pub(super) async fn retire_stopped(
    shared: &Shared,
    context: &OperationContext,
) -> Result<(), ErrorKind> {
    shared.validate(context)?;
    let previous = shared
        .retirement_resume
        .lock()
        .map_err(|_| ErrorKind::Uncertain)?
        .clone()
        .ok_or(ErrorKind::Uncertain)?;
    until(shared, context.deadline, &context.cancellation, async {
        let mut socket = connect(shared, 0).await?;
        // This cleanup exchange cannot fall back to a fresh hello or process
        // room events. A matched no_such_session or bye is required evidence.
        match until(shared, deadline(shared, 3000), &context.cancellation, read(&mut socket)).await {
            Ok(value) if value["type"].as_str() == Some("welcome") => {},
            Err(ErrorKind::Deadline) => {},
            Ok(_) => return Err(ErrorKind::Malformed),
            Err(error) => return Err(error),
        }
        let mut counter = 0;
        let id = next_id(&mut counter)?;
        write(&mut socket, json!({"id":id,"type":"hello","hello":{"version":"1.0","resumeid":previous.secret.as_str()}})).await?;
        let mut value = read(&mut socket).await?;
        if value["type"].as_str() == Some("welcome") { value = read(&mut socket).await?; }
        if value["id"].as_str() != Some(&id) { return Err(ErrorKind::WrongScope); }
        if value["type"].as_str() == Some("error") {
            if value["error"]["code"].as_str() != Some("no_such_session") {
                return Err(wire::failure(&value["error"]));
            }
        } else {
            let hello = &value["hello"];
            if value["type"].as_str() != Some("hello")
                || !matches!(hello["version"].as_str(), Some("1.0" | "2.0")) {
                return Err(ErrorKind::Malformed);
            }
            if PeerSession::parse(wire::text(&hello["sessionid"], 512)?)? != previous.session
                || hello.get("userid").is_some_and(|id| id.as_str() != Some(shared.account.user.as_str())) {
                return Err(ErrorKind::WrongScope);
            }
            // A resumed hello may rotate its secret before bye fails. Retain
            // that bounded secret for the next explicit reconciliation only.
            if let Some(secret) = hello.get("resumeid") {
                *shared.retirement_resume.lock().map_err(|_| ErrorKind::Uncertain)? = Some(Resume {
                    session: previous.session,
                    secret: Zeroizing::new(wire::text(secret, 2048)?.into()),
                });
            }
            retire(&mut socket, &mut counter).await?;
        }
        shared.invalidate_room(false);
        *shared.retirement_resume.lock().map_err(|_| ErrorKind::Uncertain)? = None;
        Ok(())
    }).await
}
enum End {
    Retired,
    Reconnect(ErrorKind),
    Terminal(ErrorKind),
}

async fn connected(
    shared: &Shared,
    socket: &mut Socket,
    commands: &mut mpsc::Receiver<Command>,
    counter: &mut u64,
) -> End {
    let mut pending: Option<PendingJoin> = None;
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut traffic_window = Instant::now();
    let mut received = 0usize;
    let mut last_received = Instant::now();
    let mut last_ping = Instant::now();
    let connectivity_epoch = shared.account.runtime.connectivity_epoch();
    let result: Result<End, ErrorKind> = async {
        loop {
            if !shared.live() { return Ok(End::Terminal(ErrorKind::Cancelled)); }
            let now = shared.account.runtime.now();
            if pending.as_ref().is_some_and(|p| p.cancel.is_cancelled() || p.reply.is_closed()
                || p.deadline.epoch != now.epoch || p.deadline.millis <= now.millis)
            {
                let p = pending.take().ok_or(ErrorKind::Uncertain)?;
                if let Ok(state) = shared.state.lock()
                    && let Some(room) = &state.room {
                    room.cancellation.cancel();
                }
                let _ = p.reply.send(Err(ErrorKind::Uncertain));
            }
            let closed = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?
                .room.as_ref().is_some_and(|r| r.cancellation.is_cancelled());
            if closed { return Ok(End::Retired); }
            if last_received.elapsed() > Duration::from_secs(60) { return Ok(End::Reconnect(ErrorKind::Deadline)); }
            if last_ping.elapsed() >= Duration::from_secs(20) {
                until(shared, deadline(shared, 3000), &shared.stop, async {
                    socket.send(Message::Ping(Vec::new().into())).await.map_err(socket_error)
                }).await?;
                last_ping = Instant::now();
            }
            tokio::select! {
                biased;
                _ = shared.account.runtime.connectivity_changed(connectivity_epoch) => return Ok(End::Reconnect(ErrorKind::Transport)),
                _ = tick.tick() => {},
                command = commands.recv() => {
                    let Some(command) = command else { return Ok(End::Terminal(ErrorKind::Cancelled)); };
                    match command {
                        Command::Join { generation, deadline: end, cancel, reply } => {
                            if pending.is_some() { let _ = reply.send(Err(ErrorKind::Busy)); continue; }
                            let value = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.purpose == RoomPurpose::Call && !state.mcu { let _ = reply.send(Err(ErrorKind::Unsupported)); continue; }
                                if room.generation != generation || room.cancellation.is_cancelled() || reply.is_closed() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                json!({"type":"room","room":{"roomid":room.token.as_str(),"sessionid":room.own.0.as_str()}})
                            };
                            let id = next_id(counter)?;
                            let mut value = value;
                            value["id"] = json!(id);
                            let sent = until(shared, end, &cancel, write(socket, value)).await;
                            if let Err(error) = sent { let _ = reply.send(Err(error)); return Err(error); }
                            pending = Some(PendingJoin { id, generation, deadline: end, cancel, reply });
                        }
                        Command::ForceMute { generation, peer, target, deadline: end, cancel, reply } => {
                            let value = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.generation != generation || !room.joined || room.cancellation.is_cancelled() || reply.is_closed() || cancel.is_cancelled() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                let Some(recipient) = room.members.iter().find(|member| member.talk == peer && peer != room.own) else {
                                    let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                };
                                let Some(subject) = room.members.iter().find(|member| member.talk == target && target != room.own) else {
                                    let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                };
                                json!({"type":"message","message":{"recipient":{"type":"session","sessionid":recipient.sfu.0.as_str()},"data":{"to":recipient.sfu.0.as_str(),"roomType":"video","type":"control","payload":{"action":"forceMute","peerId":subject.sfu.0.as_str()}}}})
                            };
                            let mut value = value;
                            value["id"] = json!(next_id(counter)?);
                            let sent = until(shared, end, &cancel, write(socket, value)).await;
                            let failed = sent.as_ref().err().copied();
                            let _ = reply.send(sent);
                            if let Some(error) = failed { return Err(error); }
                        }
                        Command::Activity { generation, peer, activity, deadline: end, cancel, reply } => {
                            let value = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.generation != generation || !room.joined || room.cancellation.is_cancelled() || reply.is_closed() || cancel.is_cancelled() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                let Some(member) = room.members.iter().find(|member| member.talk == peer && peer != room.own) else {
                                    let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                };
                                let (kind, payload) = match activity.encode() {
                                    Ok(value) => value,
                                    Err(error) => { let _ = reply.send(Err(error)); continue; }
                                };
                                json!({"type":"message","message":{"recipient":{"type":"session","sessionid":member.sfu.0.as_str()},"data":{"to":member.sfu.0.as_str(),"roomType":"video","type":kind,"payload":payload}}})
                            };
                            let mut value = value;
                            value["id"] = json!(next_id(counter)?);
                            let sent = until(shared, end, &cancel, write(socket, value)).await;
                            let failed = sent.as_ref().err().copied();
                            let _ = reply.send(sent);
                            if let Some(error) = failed { return Err(error); }
                        }
                        Command::Typing { generation, started, deadline: end, cancel, reply } => {
                            let peers = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.generation != generation || room.purpose != RoomPurpose::Chat || !room.joined
                                    || room.cancellation.is_cancelled() || reply.is_closed() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                room.members.iter().filter(|member| member.talk != room.own)
                                    .map(|member| member.sfu.clone()).collect::<Vec<_>>()
                            };
                            let mut result = Ok(());
                            for peer in peers {
                                if cancel.is_cancelled() || reply.is_closed() { result = Err(ErrorKind::Cancelled); break; }
                                let valid = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?.room.as_ref()
                                    .is_some_and(|room| room.generation == generation && room.joined && !room.cancellation.is_cancelled()
                                        && room.members.iter().any(|member| member.sfu == peer));
                                if !valid { continue; }
                                let mut value = wire::typing_message(&peer, started);
                                value["id"] = json!(next_id(counter)?);
                                if let Err(error) = until(shared, end, &cancel, write(socket, value)).await {
                                    result = Err(error); break;
                                }
                            }
                            let failed = result.as_ref().err().copied();
                            let _ = reply.send(result);
                            if let Some(error @ (ErrorKind::Transport | ErrorKind::Deadline)) = failed { return Err(error); }
                        }
                        Command::Send { generation, stream, peer, sid, signal, deadline: end, cancel, reply } => {
                            let stop_screen = matches!(&signal, Some(Signal::StopScreen));
                            let request_offer = signal.is_none();
                            let id = next_id(counter)?;
                            let value = {
                                let mut state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &mut state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.generation != generation || !room.joined || room.cancellation.is_cancelled() || reply.is_closed() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                if matches!(signal, Some(Signal::SendOffer)) && room.purpose == RoomPurpose::Call {
                                    let Some(member) = room.members.iter().find(|m| m.talk == peer && m.talk != room.own) else {
                                        let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                    };
                                    wire::stream_message(&member.sfu, None, signal, stream)
                                } else {
                                    let Some(selected) = room.selected.iter_mut().find(|p| p.talk == peer && p.sid == sid && p.stream == stream) else {
                                        let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                    };
                                    if request_offer {
                                        if room.subscription_requests.len() >= 64 { let _ = reply.send(Err(ErrorKind::Busy)); return Err(ErrorKind::Busy); }
                                        room.subscription_requests.push(id.clone());
                                        selected.request_id = Some(id.clone());
                                    }
                                    wire::stream_message(&selected.sfu, sid.as_ref(), signal, stream)
                                }
                            };
                            let mut value = value;
                            value["id"] = json!(id);
                            let mut sent = until(shared, end, &cancel, write(socket, value)).await;
                            if stop_screen && sent.is_ok() {
                                let broadcast = json!({"id":next_id(counter)?,"type":"message","message":{"recipient":{"type":"room"},"data":{"type":"unshareScreen","roomType":"screen"}}});
                                sent = until(shared, end, &cancel, write(socket, broadcast)).await;
                            }
                            let failed = sent.as_ref().err().copied();
                            let _ = reply.send(sent);
                            if let Some(error) = failed { return Err(error); }
                        }
                    }
                },
                incoming = socket.next() => {
                    if traffic_window.elapsed() >= Duration::from_secs(1) { traffic_window = Instant::now(); received = 0; }
                    received += 1;
                    if received > 256 { return Err(ErrorKind::TooLarge); }
                    last_received = Instant::now();
                    let value = match incoming {
                        Some(Ok(Message::Text(text))) => {
                            if text.len() > wire::FRAME_BYTES { return Err(ErrorKind::TooLarge); }
                            crate::json::json(text.as_bytes())?
                        }
                        Some(Ok(Message::Ping(_))) => { socket.flush().await.map_err(socket_error)?; continue; }
                        Some(Ok(Message::Pong(_))) => continue,
                        Some(Ok(Message::Close(_))) | None => return Ok(End::Reconnect(ErrorKind::Transport)),
                        Some(Err(error)) => return Err(socket_error(error)),
                        _ => return Err(ErrorKind::Malformed),
                    };
                    if let Some(p) = &pending
                        && value["id"].as_str() == Some(&p.id) {
                            let p = pending.take().ok_or(ErrorKind::Uncertain)?;
                            let outcome = if value["type"].as_str() == Some("error") {
                                Err(wire::failure(&value["error"]))
                            } else {
                                let mut state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                match state.room.as_mut() {
                                    Some(room) if room.generation == p.generation && !room.cancellation.is_cancelled()
                                        && value["type"].as_str() == Some("room")
                                        && value["room"]["roomid"].as_str() == Some(room.token.as_str()) => {
                                            room.joined = true; Ok(())
                                        }
                                    _ => Err(ErrorKind::WrongScope),
                                }
                            };
                            let failed = outcome.as_ref().err().copied();
                            let abandoned = p.reply.send(outcome).is_err();
                            if (abandoned || failed.is_some())
                                && let Ok(state) = shared.state.lock()
                                && let Some(room) = &state.room {
                                room.cancellation.cancel();
                            }
                            continue;
                    }
                    if let Err(error) = shared.incoming(&value) {
                        #[cfg(feature = "backend-test")]
                        {
                            let kind = match value["type"].as_str() { Some("event") => "event", Some("message") => "message", Some("error") => "error", Some("bye") => "bye", _ => "extension" };
                            let target = match value["event"]["target"].as_str() { Some("room") => "room", Some("participants") => "participants", Some("roomlist") => "roomlist", _ => "other" };
                            let event = match value["event"]["type"].as_str() { Some("join") => "join", Some("leave") => "leave", Some("update") => "update", Some("message") => "message", _ => "other" };
                            eprintln!("HPB incoming stopped: {kind}/{target}/{event} {error:?}");
                        }
                        return Err(error);
                    }
                }
            }
        }
    }.await;
    if let Some(p) = pending {
        if let Ok(state) = shared.state.lock()
            && let Some(room) = &state.room
        {
            room.cancellation.cancel();
        }
        let _ = p.reply.send(Err(ErrorKind::Uncertain));
    }
    match result {
        Ok(end) => end,
        Err(error @ (ErrorKind::Transport | ErrorKind::Deadline)) => End::Reconnect(error),
        Err(error) => End::Terminal(error),
    }
}
pub(super) async fn run(shared: Arc<Shared>, mut commands: mpsc::Receiver<Command>) {
    let mut resume = None;
    let mut counter = 0u64;
    let mut retries = RetryBudget::default();
    let terminal = loop {
        if !shared.live() {
            break ErrorKind::Cancelled;
        }
        let result = connect(&shared, retries.failures).await;
        let mut socket = match result {
            Ok(socket) => socket,
            Err(error @ (ErrorKind::Transport | ErrorKind::Deadline)) if retries.interrupted() => {
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                let _ = shared.interrupt();
                let wait = Duration::from_millis(250u64 << retries.failures);
                if until(
                    &shared,
                    deadline(&shared, wait.as_millis() as u64 + 100),
                    &shared.stop,
                    async {
                        tokio::time::sleep(wait).await;
                        Ok(())
                    },
                )
                .await
                .is_err()
                {
                    break ErrorKind::Cancelled;
                }
                continue;
            }
            Err(error) => break error,
        };
        if let Err(error) = authenticate(&shared, &mut socket, &mut resume, &mut counter).await {
            #[cfg(feature = "backend-test")]
            eprintln!("HPB authentication stopped: {error:?}");
            if matches!(error, ErrorKind::Transport | ErrorKind::Deadline) && retries.interrupted()
            {
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                if shared.interrupt().is_err() {
                    break ErrorKind::Busy;
                }
                while let Ok(command) = commands.try_recv() {
                    command.reject(error);
                }
                continue;
            }
            let _ = retire(&mut socket, &mut counter).await;
            break error;
        }
        let connected_at = Instant::now();
        let outcome = connected(&shared, &mut socket, &mut commands, &mut counter).await;
        retries.connected_for(connected_at.elapsed());
        match outcome {
            End::Retired => {
                match retire(&mut socket, &mut counter).await {
                    Ok(()) => {
                        shared.invalidate_room(false);
                        resume = None;
                        if let Ok(mut state) = shared.state.lock() {
                            state.session = None;
                            state.first_transport_failure = None;
                            state.status = LiveState::Connecting;
                        }
                        // A fresh account event session is allowed; no room or
                        // call is recreated. A new call needs HTTP admission.
                    }
                    Err(_) => break ErrorKind::Uncertain,
                }
            }
            End::Reconnect(error) if retries.interrupted() => {
                #[cfg(feature = "backend-test")]
                eprintln!("HPB transport interrupted: {error:?}");
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                if shared.interrupt().is_err() {
                    break ErrorKind::Busy;
                }
                // No command is replayed on reconnect. Any abandoned write
                // cancels its route, so only unchanged live sessions can resume.
                while let Ok(command) = commands.try_recv() {
                    command.reject(ErrorKind::Transport);
                }
            }
            End::Reconnect(error) | End::Terminal(error) => {
                let _ = retire(&mut socket, &mut counter).await;
                break error;
            }
        }
    };
    #[cfg(feature = "backend-test")]
    eprintln!("HPB session stopped: {terminal:?}");
    if matches!(
        terminal,
        ErrorKind::Transport | ErrorKind::Deadline | ErrorKind::Uncertain
    ) && let Ok(mut retained) = shared.retirement_resume.lock()
    {
        *retained = resume;
    }
    // Terminal policy/auth errors take precedence as the current state; the
    // session-scoped first transport failure remains a separate safe diagnostic.
    shared.status(LiveState::Stopped(terminal));
    while let Ok(command) = commands.try_recv() {
        command.reject(terminal);
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn separate_healthy_episodes_do_not_exhaust_a_long_lived_account() {
        let mut budget = RetryBudget::default();
        for _ in 0..100 {
            assert!(budget.interrupted());
            assert!(budget.interrupted());
            budget.connected_for(Duration::from_secs(30));
        }
        assert!(budget.interrupted());
    }

    #[test]
    fn handshakes_and_short_flaps_cannot_renew_the_bounded_retry_window() {
        let mut budget = RetryBudget::default();
        for _ in 0..4 {
            assert!(budget.interrupted());
            budget.connected_for(Duration::from_millis(29_999));
        }
        assert!(!budget.interrupted());
        assert!(!budget.interrupted());
        assert_eq!(budget.failures, 4);
    }
}
