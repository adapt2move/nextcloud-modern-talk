use super::*;
use crate::scope::{LeaseId, NegotiationId, RoomKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
const ROUTE: &str = "ocs/v2.php/apps/spreed/api/v3/signaling/testroom";
fn scope(id: u128) -> SessionScope {
    SessionScope {
        account: BindingId::from_key(id),
        runtime: LeaseId::from_key(1),
    }
}
fn call(scope: SessionScope, generation: u128) -> CallKey {
    CallKey {
        account: scope,
        conversation: RoomKey {
            account: scope.account,
            host: OriginId::from_key(1),
            local: 1,
        },
        call: NegotiationId::from_key(generation),
    }
}
fn runtime() -> NetworkRuntime {
    NetworkRuntime::new(ClockId::from_key(1))
}
fn context(t: &Transport) -> OperationContext {
    OperationContext {
        scope: t.scope(),
        deadline: Moment {
            epoch: t.now().epoch,
            millis: t.now().millis + 5000,
        },
        cancellation: Cancellation::default(),
    }
}
fn transport(r: &NetworkRuntime, id: u128, origin: &str) -> Result<Transport, PortError> {
    let random = id.to_le_bytes();
    Transport::new(
        AuthenticatedBinding {
            scope: scope(id),
            host: OriginId::from_key(1),
            origin: ServerOrigin::loopback(origin).unwrap(),
            login_name: Zeroizing::new("synthetic-user".into()),
            user: Zeroizing::new("synthetic-user".into()),
            password: Zeroizing::new(random.iter().map(|v| format!("{v:02x}")).collect()),
            cancellation: Cancellation::default(),
        },
        r.clone(),
    )
}
fn open(t: &mut Transport, generation: u128) -> (CallKey, DiagnosticLane, DiagnosticLane) {
    let ctx = context(t);
    let scope = call(t.scope(), generation);
    let mut admission = t.diagnostic_call_admission(&ctx, scope).unwrap();
    let (reader, writer) = t.diagnostic_duplex(&ctx, &mut admission).unwrap();
    (scope, reader, writer)
}
#[test]
fn atomic_reservation_rollback_and_account_retirement_use_original_charges() {
    let r = runtime();
    let mut a = transport(&r, 1, "http://127.0.0.1:9/").unwrap();
    let ctx = context(&a);
    let c = call(a.scope(), 1);
    let mut token = a.diagnostic_call_admission(&ctx, c).unwrap();
    let seven = r.0.requests.clone().try_acquire_many_owned(7).unwrap();
    assert!(matches!(a.diagnostic_duplex(&ctx,&mut token),Err(e)if e.kind==ErrorKind::Busy));
    assert!(!token.duplex_started);
    assert_eq!(r.0.requests.available_permits(), 1);
    assert!(
        r.0.diagnostic_loops
            .lock()
            .unwrap()
            .iter()
            .all(Option::is_none)
    );
    drop(seven);
    let (ar, aw) = a.diagnostic_duplex(&ctx, &mut token).unwrap();
    assert!(matches!(a.diagnostic_duplex(&ctx,&mut token),Err(e)if e.kind==ErrorKind::Conflict));
    let mut b = transport(&r, 2, "http://127.0.0.1:9/").unwrap();
    let (_, br, bw) = open(&mut b, 2);
    assert_eq!(r.0.requests.available_permits(), 4);
    let four = r.0.requests.clone().try_acquire_many_owned(4).unwrap();
    assert!(r.0.requests.clone().try_acquire_owned().is_err());
    let mut c = transport(&r, 3, "http://127.0.0.1:9/").unwrap();
    let ctx = context(&c);
    let mut token = c
        .diagnostic_call_admission(&ctx, call(c.scope(), 3))
        .unwrap();
    assert!(matches!(c.diagnostic_duplex(&ctx,&mut token),Err(e)if e.kind==ErrorKind::Busy));
    drop(a);
    assert!(matches!(transport(&r,1,"http://127.0.0.1:9/"),Err(e)if e.kind==ErrorKind::Conflict));
    drop(ar);
    assert!(matches!(transport(&r,1,"http://127.0.0.1:9/"),Err(e)if e.kind==ErrorKind::Conflict));
    assert_eq!(r.0.requests.available_permits(), 0);
    drop(aw);
    assert_eq!(r.0.requests.available_permits(), 2);
    drop(transport(&r, 1, "http://127.0.0.1:9/").unwrap());
    drop(b);
    drop(bw);
    assert!(matches!(transport(&r,2,"http://127.0.0.1:9/"),Err(e)if e.kind==ErrorKind::Conflict));
    drop(br);
    drop(four);
    assert_eq!(r.0.requests.available_permits(), 8);
    drop(transport(&r, 2, "http://127.0.0.1:9/").unwrap());
}
struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<String>>>,
    release: Arc<tokio::sync::Notify>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(hold_get: bool, headers: &'static str, body: &'static str) -> Self {
        Self::split(if hold_get { "GET" } else { "" }, headers, headers, body).await
    }
    async fn split(
        hold: &'static str,
        read_headers: &'static str,
        write_headers: &'static str,
        body: &'static str,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let output = seen.clone();
        let release = Arc::new(tokio::sync::Notify::new());
        let signal = release.clone();
        let task = tokio::spawn(async move {
            let mut tasks = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted=listener.accept()=>{
                        let (mut stream,_)=accepted.unwrap();let output=output.clone();let signal=signal.clone();
                        tasks.spawn(async move {
                            let mut request=Zeroizing::new(Vec::new());
                            loop {
                                let mut bytes=[0;1024];let n=stream.read(&mut bytes).await.unwrap_or(0);if n==0{return;}
                                if request.len()+n>BODY_LIMIT+65536{return;}request.extend_from_slice(&bytes[..n]);
                                if let Some(end)=request.windows(4).position(|p|p==b"\r\n\r\n") {
                                    let text=std::str::from_utf8(&request[..end]).unwrap();
                                    let length=text.lines().find_map(|l|l.to_ascii_lowercase().strip_prefix("content-length:").and_then(|v|v.trim().parse::<usize>().ok())).unwrap_or(0);
                                    if request.len()>=end+4+length {break;}
                                }
                            }
                            let method=std::str::from_utf8(&request).unwrap().split_whitespace().next().unwrap().to_owned();
                            output.lock().unwrap().push(method.clone());drop(request);
                            if hold==method {signal.notified().await;}
                            let headers=if method=="GET" {read_headers}else{write_headers};
                            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",body.len(),headers,body);
                            let _=stream.write_all(response.as_bytes()).await;
                        });
                    }
                    _=tasks.join_next(),if !tasks.is_empty()=>{},
                }
            }
        });
        Self {
            origin,
            seen,
            release,
            task,
        }
    }
    async fn count(&self, n: usize) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.seen.lock().unwrap().len() < n {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
    }
}
const OK: &str = r#"{"ocs":{"meta":{"status":"ok","statuscode":200},"data":[]}}"#;
#[tokio::test]
async fn reserved_outbound_finishes_during_get_without_third_acquire_then_leave_after_stop() {
    let server = Server::start(true, "", OK).await;
    let r = runtime();
    let mut t = transport(&r, 1, &server.origin).unwrap();
    let (call, reader, writer) = open(&mut t, 1);
    let ctx = context(&t);
    let six = r.0.requests.clone().try_acquire_many_owned(6).unwrap();
    let prepared = t
        .diagnostic_prepare(
            &ctx,
            call,
            reader,
            DiagnosticOperation::Read { route: ROUTE },
        )
        .unwrap();
    let reading = tokio::spawn(prepared.wait());
    server.count(1).await;
    assert!(
        matches!(t.ocs(&ctx,Method::GET,"ocs/v2.php/cloud/user",&[],&[],false).await,Err(e)if e.kind==ErrorKind::Busy)
    );
    let writing = t
        .diagnostic_prepare(
            &ctx,
            call,
            writer,
            DiagnosticOperation::Signal {
                route: ROUTE,
                messages: "[]",
            },
        )
        .unwrap()
        .wait()
        .await;
    let (writer, result) = t.diagnostic_commit(&ctx, call, writing);
    assert!(result.is_ok());
    assert!(!reading.is_finished());
    t.diagnostic_stop();
    let read = reading.await.unwrap();
    let reader = read.discard();
    let leave = t
        .diagnostic_prepare(
            &ctx,
            call,
            writer,
            DiagnosticOperation::Leave {
                route: "ocs/v2.php/apps/spreed/api/v4/call/testroom",
            },
        )
        .unwrap()
        .wait()
        .await;
    let (writer, result) = t.diagnostic_commit(&ctx, call, leave);
    assert!(result.is_ok());
    assert_eq!(*server.seen.lock().unwrap(), vec!["GET", "POST", "DELETE"]);
    assert!(
        matches!(t.diagnostic_prepare(&ctx,call,writer,DiagnosticOperation::Signal{route:ROUTE,messages:"[]"}),Err(e)if e.kind==ErrorKind::Uncertain)
    );
    assert_eq!(r.0.requests.available_permits(), 0);
    drop(reader);
    assert_eq!(r.0.requests.available_permits(), 2);
    drop(six);
}
#[tokio::test]
async fn cancellation_drop_stale_and_discard_never_commit_or_replay() {
    for phase in 0..5 {
        let server = Server::start(
            phase == 2,
            "Set-Cookie: sid=private; Path=/; HttpOnly\r\n",
            OK,
        )
        .await;
        let r = runtime();
        let mut t = transport(&r, 1, &server.origin).unwrap();
        let (call, reader, writer) = open(&mut t, 1);
        let ctx = context(&t);
        let prepared = t
            .diagnostic_prepare(
                &ctx,
                call,
                reader,
                DiagnosticOperation::Read { route: ROUTE },
            )
            .unwrap();
        match phase {
            0 => {
                drop(prepared);
                assert_eq!(server.seen.lock().unwrap().len(), 0);
            }
            1 => {
                ctx.cancellation.cancel();
                let completion = prepared.wait().await;
                let (reader, result) = t.diagnostic_commit(&ctx, call, completion);
                assert!(matches!(result,Err(e)if e.kind==ErrorKind::Cancelled));
                drop(reader);
                assert_eq!(server.seen.lock().unwrap().len(), 0);
            }
            2 => {
                let reading = tokio::spawn(prepared.wait());
                server.count(1).await;
                reading.abort();
                assert!(reading.await.is_err());
            }
            3 => {
                let completion = prepared.wait().await;
                let stale = CallKey {
                    call: NegotiationId::from_key(2),
                    ..call
                };
                let (reader, result) = t.diagnostic_commit(&ctx, stale, completion);
                assert!(matches!(result,Err(e)if e.kind==ErrorKind::WrongScope));
                drop(reader);
            }
            _ => {
                let completion = prepared.wait().await;
                let reader = completion.discard();
                drop(reader);
            }
        }
        assert!(t.cookies.is_empty());
        assert!(
            matches!(t.diagnostic_prepare(&context(&t),call,writer,DiagnosticOperation::Signal{route:ROUTE,messages:"[]"}),Err(e)if matches!(e.kind,ErrorKind::Uncertain|ErrorKind::Cancelled))
        );
        assert_eq!(r.0.requests.available_permits(), 8);
    }
}
#[tokio::test]
async fn prepared_and_completed_auth_keep_binding_until_both_retirement_orders_settle() {
    for read_first in [true, false] {
        let server = Server::start(false, "", OK).await;
        let r = runtime();
        let mut t = transport(&r, 1, &server.origin).unwrap();
        let (call, reader, writer) = open(&mut t, 1);
        let ctx = context(&t);
        let read = t
            .diagnostic_prepare(
                &ctx,
                call,
                reader,
                DiagnosticOperation::Read { route: ROUTE },
            )
            .unwrap();
        let write = t
            .diagnostic_prepare(
                &ctx,
                call,
                writer,
                DiagnosticOperation::Signal {
                    route: ROUTE,
                    messages: "[]",
                },
            )
            .unwrap()
            .wait()
            .await;
        let response = t.diagnostic_response_slot().unwrap();
        drop(t);
        assert!(matches!(transport(&r,1,&server.origin),Err(e)if e.kind==ErrorKind::Conflict));
        if read_first {
            drop(read);
            assert!(matches!(transport(&r,1,&server.origin),Err(e)if e.kind==ErrorKind::Conflict));
            drop(write);
        } else {
            drop(write);
            assert!(matches!(transport(&r,1,&server.origin),Err(e)if e.kind==ErrorKind::Conflict));
            drop(read);
        }
        assert_eq!(r.0.requests.available_permits(), 8);
        drop(transport(&r, 1, &server.origin).unwrap());
        assert_eq!(r.0.diagnostic_responses.available_permits(), 7);
        drop(response);
        assert_eq!(r.0.diagnostic_responses.available_permits(), 8);
    }
}
fn delta(values: &[&str], now: Instant, wall: i64) -> Result<Vec<CookieMutation>, ErrorKind> {
    let mut headers = reqwest::header::HeaderMap::new();
    for value in values {
        headers.append(SET_COOKIE, HeaderValue::from_str(value).unwrap());
    }
    cookie_delta(&headers, "/", now, wall)
}
#[test]
fn cookie_transaction_idempotence_requires_expiry_attributes_deletion_and_one_revision() {
    let now = Instant::now();
    let wall = cookie::time::OffsetDateTime::now_utc().unix_timestamp();
    let a = delta(&["sid=one; Path=/; HttpOnly; SameSite=Lax"], now, wall).unwrap();
    let mut order = CookieOrder::default();
    let mut jar = Vec::new();
    order.commit(&mut jar, 2, 0, a.clone()).unwrap();
    order.commit(&mut jar, 1, 0, a.clone()).unwrap();
    assert_eq!(order.revision, 1);
    for value in [
        "sid=one; Path=/; SameSite=Lax",
        "sid=one; Path=/; HttpOnly; SameSite=Strict",
        "sid=one; Path=/; HttpOnly; SameSite=Lax; Secure",
        "sid=two; Path=/; HttpOnly; SameSite=Lax",
    ] {
        assert_eq!(
            order.commit(&mut jar, 1, 0, delta(&[value], now, wall).unwrap()),
            Err(ErrorKind::Uncertain)
        );
        assert!(jar[0].value.as_str() == "one");
    }
    assert_eq!(
        order.commit(&mut jar, 1, 0, Vec::new()),
        Err(ErrorKind::Uncertain)
    );
    assert_eq!(
        order.commit(&mut jar, 1, 1, a.clone()),
        Err(ErrorKind::Uncertain)
    );
    let deleted = delta(&["sid=discarded; Path=/; Max-Age=0"], now, wall).unwrap();
    order.commit(&mut jar, 3, 1, deleted).unwrap();
    assert!(jar.is_empty());
    let same_delete = delta(
        &["sid=other; Path=/; Expires=Thu, 01 Jan 1970 00:00:00 GMT"],
        now,
        wall,
    )
    .unwrap();
    order.commit(&mut jar, 2, 1, same_delete).unwrap();
    assert_eq!(order.commit(&mut jar, 1, 0, a), Err(ErrorKind::Uncertain));
    let relative = delta(&["sid=next; Path=/; Max-Age=60"], now, wall).unwrap();
    order.commit(&mut jar, 4, 2, relative.clone()).unwrap();
    assert_eq!(
        order.commit(
            &mut jar,
            3,
            2,
            delta(
                &["sid=next; Path=/; Max-Age=60"],
                now + Duration::from_millis(1),
                wall
            )
            .unwrap()
        ),
        Err(ErrorKind::Uncertain)
    );
    order.commit(&mut jar, 3, 2, relative).unwrap();
    let expires = jar[0].expires;
    order.revision = u64::MAX;
    assert_eq!(
        order.commit(
            &mut jar,
            5,
            u64::MAX,
            delta(&["sid=x; Path=/"], now, wall).unwrap()
        ),
        Err(ErrorKind::Uncertain)
    );
    assert_eq!(jar[0].expires, expires);
}
#[test]
fn cookie_full_delta_is_bounded_and_failed_final_mutation_is_atomic() {
    let now = Instant::now();
    let wall = cookie::time::OffsetDateTime::now_utc().unix_timestamp();
    for values in [
        vec!["sid=one; Path=/", "sid=two; Path=/"],
        vec!["sid=x; Domain=example.invalid"],
        vec!["sid=x; HttpOnly=0"],
        vec!["sid=x; Unknown=y"],
        vec!["sid=x; Max-Age=bad"],
        vec!["sid=x; SameSite=bad"],
        vec!["sid=x; Expires=bad"],
    ] {
        assert!(delta(&values, now, wall).is_err());
    }
    let nine: Vec<_> = (0..9).map(|n| format!("k{n}=v; Path=/")).collect();
    let refs: Vec<_> = nine.iter().map(String::as_str).collect();
    assert!(matches!(delta(&refs, now, wall), Err(ErrorKind::TooLarge)));
    let mut jar = Vec::new();
    let mut order = CookieOrder::default();
    order
        .commit(&mut jar, 1, 0, delta(&refs[..8], now, wall).unwrap())
        .unwrap();
    assert_eq!(
        order.commit(
            &mut jar,
            2,
            1,
            delta(&["k0=changed; Path=/", "k8=v; Path=/"], now, wall).unwrap()
        ),
        Err(ErrorKind::TooLarge)
    );
    assert_eq!(order.revision, 1);
    assert!(jar.iter().all(|c| c.value.as_str() == "v"));
}
#[tokio::test]
async fn malformed_response_and_exhaustion_never_update_cookie_or_issue_more_io() {
    let server = Server::start(false, "Set-Cookie: sid=private; Path=/\r\n", "invalid").await;
    let r = runtime();
    let mut t = transport(&r, 1, &server.origin).unwrap();
    let (call, reader, writer) = open(&mut t, 1);
    let ctx = context(&t);
    let completion = t
        .diagnostic_prepare(
            &ctx,
            call,
            reader,
            DiagnosticOperation::Read { route: ROUTE },
        )
        .unwrap()
        .wait()
        .await;
    let (reader, result) = t.diagnostic_commit(&ctx, call, completion);
    assert!(result.is_err());
    assert!(t.cookies.is_empty());
    drop(reader);
    drop(writer);
    let (call, reader, writer) = open(&mut t, 2);
    t.diagnostic_cookies.next = u64::MAX;
    assert!(
        matches!(t.diagnostic_prepare(&ctx,call,reader,DiagnosticOperation::Read{route:ROUTE}),Err(e)if e.kind==ErrorKind::Uncertain)
    );
    drop(writer);
    assert_eq!(server.seen.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn real_overlap_commits_only_identical_normalized_cookie_delta() {
    for identical in [true, false] {
        let read_headers = if identical {
            "Set-Cookie: sid=one; Path=/; HttpOnly\r\n"
        } else {
            "Set-Cookie: sid=one; Path=/\r\n"
        };
        let server = Server::split(
            "GET",
            read_headers,
            "Set-Cookie: sid=one; Path=/; HttpOnly\r\n",
            OK,
        )
        .await;
        let r = runtime();
        let mut t = transport(&r, 1, &server.origin).unwrap();
        let (call, reader, writer) = open(&mut t, 1);
        let ctx = context(&t);
        let read = tokio::spawn(
            t.diagnostic_prepare(
                &ctx,
                call,
                reader,
                DiagnosticOperation::Read { route: ROUTE },
            )
            .unwrap()
            .wait(),
        );
        server.count(1).await;
        let write = t
            .diagnostic_prepare(
                &ctx,
                call,
                writer,
                DiagnosticOperation::Signal {
                    route: ROUTE,
                    messages: "[]",
                },
            )
            .unwrap()
            .wait()
            .await;
        let (writer, result) = t.diagnostic_commit(&ctx, call, write);
        assert!(result.is_ok());
        assert_eq!(t.diagnostic_cookies.revision, 1);
        server.release.notify_one();
        let completion = read.await.unwrap();
        let (reader, result) = t.diagnostic_commit(&ctx, call, completion);
        assert_eq!(result.is_ok(), identical);
        if !identical {
            assert!(matches!(result,Err(e)if e.kind==ErrorKind::Uncertain));
        }
        assert_eq!(t.diagnostic_cookies.revision, 1);
        assert!(t.cookies[0].value.as_str() == "one");
        drop(reader);
        drop(writer);
    }
}
#[tokio::test]
async fn stop_during_dispatched_write_settles_then_terminal_leave_and_identity_cancel_refuses_io() {
    let server = Server::split("POST", "", "", OK).await;
    let r = runtime();
    let mut t = transport(&r, 1, &server.origin).unwrap();
    let (call, reader, writer) = open(&mut t, 1);
    let ctx = context(&t);
    let writing = tokio::spawn(
        t.diagnostic_prepare(
            &ctx,
            call,
            writer,
            DiagnosticOperation::Signal {
                route: ROUTE,
                messages: "[]",
            },
        )
        .unwrap()
        .wait(),
    );
    server.count(1).await;
    t.diagnostic_stop();
    assert!(!writing.is_finished());
    server.release.notify_one();
    let (writer, result) = t.diagnostic_commit(&ctx, call, writing.await.unwrap());
    assert!(matches!(result,Err(e)if e.kind==ErrorKind::Uncertain));
    let leave = t
        .diagnostic_prepare(
            &ctx,
            call,
            writer,
            DiagnosticOperation::Leave {
                route: "ocs/v2.php/apps/spreed/api/v4/call/testroom",
            },
        )
        .unwrap()
        .wait()
        .await;
    let (writer, result) = t.diagnostic_commit(&ctx, call, leave);
    assert!(result.is_ok());
    drop(reader);
    drop(writer);
    let (call, reader, writer) = open(&mut t, 2);
    t.cancellation().cancel();
    assert!(
        matches!(t.diagnostic_prepare(&context(&t),call,writer,DiagnosticOperation::Leave{route:"ocs/v2.php/apps/spreed/api/v4/call/testroom"}),Err(e)if e.kind==ErrorKind::Cancelled)
    );
    drop(reader);
    assert_eq!(*server.seen.lock().unwrap(), vec!["POST", "DELETE"]);
}
#[tokio::test]
async fn invalid_context_and_cross_call_prepare_are_rejected_without_io() {
    let server = Server::start(false, "", OK).await;
    let r = runtime();
    let mut t = transport(&r, 1, &server.origin).unwrap();
    let ctx = context(&t);
    let call = call(t.scope(), 1);
    let mut token = t.diagnostic_call_admission(&ctx, call).unwrap();
    let cancelled = context(&t);
    cancelled.cancellation.cancel();
    assert!(
        matches!(t.diagnostic_duplex(&cancelled,&mut token),Err(e)if e.kind==ErrorKind::Cancelled)
    );
    assert!(!token.duplex_started);
    let mut expired = context(&t);
    expired.deadline.millis = 0;
    assert!(
        matches!(t.diagnostic_duplex(&expired,&mut token),Err(e)if e.kind==ErrorKind::Deadline)
    );
    assert!(!token.duplex_started);
    let (reader, writer) = t.diagnostic_duplex(&ctx, &mut token).unwrap();
    let stale = CallKey {
        call: NegotiationId::from_key(2),
        ..call
    };
    assert!(
        matches!(t.diagnostic_prepare(&ctx,stale,reader,DiagnosticOperation::Read{route:ROUTE}),Err(e)if e.kind==ErrorKind::WrongScope)
    );
    drop(writer);
    assert!(server.seen.lock().unwrap().is_empty());
    assert_eq!(r.0.requests.available_permits(), 8);
}
