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
struct PendingJoin {
    id: String,
    generation: u64,
    deadline: Moment,
    cancel: Cancellation,
    reply: oneshot::Sender<Result<(), ErrorKind>>,
}
struct Resume {
    session: PeerSession,
    secret: Zeroizing<String>,
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
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
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
                shared.invalidate_room();
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
                _ = tick.tick() => {},
                command = commands.recv() => {
                    let Some(command) = command else { return Ok(End::Terminal(ErrorKind::Cancelled)); };
                    match command {
                        Command::Join { generation, deadline: end, cancel, reply } => {
                            if pending.is_some() { let _ = reply.send(Err(ErrorKind::Busy)); continue; }
                            let value = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                if !state.mcu { let _ = reply.send(Err(ErrorKind::Unsupported)); continue; }
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
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
                        Command::Send { generation, peer, sid, signal, deadline: end, cancel, reply } => {
                            let value = {
                                let state = shared.state.lock().map_err(|_| ErrorKind::Uncertain)?;
                                let Some(room) = &state.room else { let _ = reply.send(Err(ErrorKind::WrongScope)); continue; };
                                if room.generation != generation || !room.joined || room.cancellation.is_cancelled() || reply.is_closed() {
                                    let _ = reply.send(Err(ErrorKind::Cancelled)); continue;
                                }
                                let Some(selected) = room.selected.iter().find(|p| p.talk == peer && p.sid == sid) else {
                                    let _ = reply.send(Err(ErrorKind::WrongScope)); continue;
                                };
                                wire::message(&selected.sfu, sid.as_ref(), signal)
                            };
                            let mut value = value;
                            value["id"] = json!(next_id(counter)?);
                            let sent = until(shared, end, &cancel, write(socket, value)).await;
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
                    shared.incoming(&value)?;
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
    let mut failures = 0usize;
    let terminal = loop {
        if !shared.live() {
            break ErrorKind::Cancelled;
        }
        let result = connect(&shared, failures).await;
        let mut socket = match result {
            Ok(socket) => socket,
            Err(error @ (ErrorKind::Transport | ErrorKind::Deadline)) if failures < 4 => {
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                failures += 1;
                let _ = shared.interrupt();
                let wait = Duration::from_millis(250u64 << failures);
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
            if matches!(error, ErrorKind::Transport | ErrorKind::Deadline) && failures < 4 {
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                failures += 1;
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
        match connected(&shared, &mut socket, &mut commands, &mut counter).await {
            End::Retired => {
                match retire(&mut socket, &mut counter).await {
                    Ok(()) => {
                        shared.invalidate_room();
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
            End::Reconnect(error) if failures < 4 => {
                if let Ok(mut state) = shared.state.lock() {
                    state.first_transport_failure.get_or_insert(error);
                }
                failures += 1;
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
    // Terminal policy/auth errors take precedence as the current state; the
    // session-scoped first transport failure remains a separate safe diagnostic.
    shared.status(LiveState::Stopped(terminal));
    while let Ok(command) = commands.try_recv() {
        command.reject(terminal);
    }
}
