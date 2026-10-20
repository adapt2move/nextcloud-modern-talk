//! I2 mailbox tests. Synthetic wire strings; never copied session/SDP evidence.
use super::*;
use crate::nextcloud::transport::{AuthenticatedBinding, ServerOrigin};
use crate::nextcloud::{ActiveConversation, NetworkRuntime};
use std::sync::atomic::AtomicUsize;
use talk_core::ids::{
    AccountId, AccountScope, CallGeneration, ClockEpoch, HostId, RuntimeGeneration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

pub(super) struct Server {
    pub(super) origin: String,
    count: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    pub(super) async fn new(replies: Vec<Value>) -> Self {
        assert!(replies.len() <= 32);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let task = tokio::spawn(async move {
            for value in replies {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0u8; 1024];
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    assert!(request.len() + n <= 8192);
                    request.extend_from_slice(&buf[..n]);
                    if request.windows(4).any(|x| x == b"\r\n\r\n") {
                        break;
                    }
                }
                seen.fetch_add(1, Ordering::Relaxed);
                let body = serde_json::to_vec(
                    &json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":value}}),
                )
                .unwrap();
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(header.as_bytes()).await;
                let _ = socket.write_all(&body).await;
            }
        });
        Self {
            origin,
            count,
            task,
        }
    }
    pub(super) fn count(&self) -> usize {
        self.count.load(Ordering::Relaxed)
    }
}
pub(super) fn runtime() -> NetworkRuntime {
    NetworkRuntime::new(ClockEpoch::from_local(9))
}
pub(super) fn context(runtime: &NetworkRuntime, account: u128) -> OperationContext {
    let mut deadline = runtime.now();
    deadline.millis += 2500;
    OperationContext {
        scope: AccountScope {
            account: AccountId::from_local(account),
            runtime: RuntimeGeneration::from_local(2),
        },
        deadline,
        cancellation: Cancellation::default(),
    }
}
pub(super) fn owner(origin: &str, runtime: NetworkRuntime, account: u128) -> NextcloudClient {
    NextcloudClient::from_identity(
        AuthenticatedBinding {
            scope: context(&runtime, account).scope,
            host: HostId::from_local(3),
            origin: ServerOrigin::loopback(origin).unwrap(),
            login_name: Zeroizing::new("synthetic".into()),
            user: Zeroizing::new("synthetic".into()),
            password: Zeroizing::new("unit-only".into()),
            cancellation: Cancellation::default(),
        },
        runtime,
    )
    .unwrap()
}
pub(super) fn admit(
    owner: &mut NextcloudClient,
    ctx: &OperationContext,
    generation: u128,
) -> CallScope {
    let conversation = owner.bind_room(&json!({"token":"testroom"})).unwrap();
    let call = CallScope {
        account: ctx.scope,
        conversation,
        call: CallGeneration::from_local(generation),
    };
    owner.active = Some(ActiveConversation {
        id: conversation,
        session: Zeroizing::new("local".into()),
    });
    super::admit_fixture(owner, ctx, call, "peer", "sid");
    call
}
pub(super) fn event(kind: &str, sid: Option<&str>, payload: Value) -> Value {
    let mut inner =
        json!({"from":"peer","to":"local","roomType":"video","type":kind,"payload":payload});
    if let Some(sid) = sid {
        inner["sid"] = json!(sid);
    }
    json!({"type":"message","data":inner.to_string()})
}
pub(super) fn ice() -> Value {
    event(
        "candidate",
        Some("sid"),
        json!({"candidate":{"candidate":"candidate:1 1 udp 1 127.0.0.1 9000 typ host","sdpMid":"0","sdpMLineIndex":0}}),
    )
}
pub(super) fn desc(kind: &str) -> Value {
    event(kind, Some("sid"), json!({"type":kind,"sdp":"v=0\r\n"}))
}
pub(super) fn member(session: &str) -> Value {
    json!({"roomId":7,"sessionId":session,"lastPing":1,"inCall":7,"participantPermissions":511,"userId":"unit","actorType":"users","actorId":"unit"})
}
pub(super) fn snapshot(users: Vec<Value>) -> Value {
    json!({"type":"usersInRoom","data":users})
}
async fn failed(owner: &mut NextcloudClient, ctx: &OperationContext, call: CallScope) -> ErrorKind {
    owner
        .diagnostic_poll(ctx, call)
        .await
        .err()
        .expect("expected rejection")
        .kind
}

#[tokio::test]
async fn cumulative_duplicates_across_pages_responses_and_last_page_retention() {
    let server = Server::new(vec![
        json!((0..40).map(|_| ice()).collect::<Vec<_>>()),
        json!((0..24).map(|_| ice()).collect::<Vec<_>>()),
        json!([ice()]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    let first = owner.diagnostic_poll(&ctx, call).await.unwrap();
    assert_eq!(first.inspect().unwrap().signals.len(), 32);
    assert!(first.inspect().unwrap().has_more);
    let last = owner.diagnostic_poll(&ctx, call).await.unwrap();
    assert_eq!(last.inspect().unwrap().signals.len(), 8);
    assert!(!last.inspect().unwrap().has_more);
    assert_eq!(server.count(), 1);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Busy);
    drop(first);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Busy);
    assert_eq!(server.count(), 1);
    // Moving the wrapper does not shed its guard, even with has_more=false.
    let moved = last;
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Busy);
    drop(moved);
    let second = owner.diagnostic_poll(&ctx, call).await.unwrap();
    assert_eq!(second.inspect().unwrap().signals.len(), 24);
    drop(second);
    assert_eq!(owner.diagnostic_signaling.candidates, 64);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::TooLarge);
    assert_eq!(server.count(), 3);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Conflict);
    assert_eq!(server.count(), 3);
    assert!(owner.diagnostic_signaling.mailbox.is_none());
    assert!(owner.diagnostic_signaling.response.upgrade().is_none());
}

#[tokio::test]
async fn malformed_final_row_never_dispatches_prefix_and_burns_negotiation() {
    let mut rows = vec![ice(); 40];
    rows.push(event(
        "answer",
        None,
        json!({"type":"answer","sdp":"v=0\r\n"}),
    ));
    let server = Server::new(vec![json!(rows)]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Malformed);
    assert!(owner.diagnostic_signaling.mailbox.is_none());
    assert!(owner.diagnostic_signaling.response.upgrade().is_none());
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Conflict);
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn descriptions_duplicate_budget_sid_rollover_and_unsupported_events() {
    for rows in [
        json!([desc("answer"), desc("answer"), desc("answer")]),
        json!([
            ice(),
            event("candidate", Some("other"), json!({"candidate":{}}))
        ]),
        json!([ice(), event("unknown", None, json!({}))]),
        json!(
            (0..17)
                .map(|_| event("unmute", None, json!({"name":"audio"})))
                .collect::<Vec<_>>()
        ),
        json!([snapshot(vec![]), snapshot(vec![]), snapshot(vec![])]),
    ] {
        let server = Server::new(vec![rows]).await;
        let rt = runtime();
        let ctx = context(&rt, 1);
        let mut owner = owner(&server.origin, rt, 1);
        let call = admit(&mut owner, &ctx, 1);
        assert!(matches!(
            failed(&mut owner, &ctx, call).await,
            ErrorKind::TooLarge | ErrorKind::WrongScope | ErrorKind::Unsupported
        ));
        assert!(owner.diagnostic_signaling.mailbox.is_none());
        assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Conflict);
        assert_eq!(server.count(), 1);
    }
    let server = Server::new(vec![
        json!([desc("answer")]),
        json!([desc("answer")]),
        json!([desc("answer")]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    drop(owner.diagnostic_poll(&ctx, call).await.unwrap());
    drop(owner.diagnostic_poll(&ctx, call).await.unwrap());
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::TooLarge);
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn membership_last_snapshot_same_provenance_every_page_and_absence_is_explicit() {
    let mut rows = vec![snapshot(vec![member("peer")])];
    rows.extend(vec![ice(); 40]);
    rows.push(snapshot(vec![]));
    let server = Server::new(vec![json!(rows), json!([])]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    let first = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let last = owner.diagnostic_poll(&ctx, call).await.unwrap();
    for page in [&first, &last] {
        let view = page.inspect().unwrap();
        assert!(view.membership_observed);
        assert_eq!(view.selected_peer_present, Some(false));
        assert_eq!(view.peer_count, 0);
    }
    assert_eq!(
        first.inspect().unwrap().request_started,
        last.inspect().unwrap().request_started
    );
    drop(last);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Busy);
    drop(first);
    let empty = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let view = empty.inspect().unwrap();
    assert!(!view.membership_observed);
    assert_eq!(view.selected_peer_present, None);
    assert!(view.signals.is_empty());
}

#[tokio::test]
async fn stale_cancel_deadline_and_client_retirement_invalidate_old_pages_without_releasing_guard()
{
    for fault in 0..4 {
        let server = Server::new(vec![json!(vec![ice(); 40])]).await;
        let rt = runtime();
        let mut ctx = context(&rt, 1);
        let mut owner = owner(&server.origin, rt.clone(), 1);
        let call = admit(&mut owner, &ctx, 1);
        let page = owner.diagnostic_poll(&ctx, call).await.unwrap();
        if fault == 3 {
            drop(owner);
            assert!(page.inspect().is_err());
            drop(page);
            continue;
        }
        let mut target = call;
        match fault {
            0 => target.call = CallGeneration::from_local(99),
            1 => ctx.cancellation.cancel(),
            _ => ctx.deadline = rt.now(),
        }
        assert!(matches!(
            failed(&mut owner, &ctx, target).await,
            ErrorKind::WrongScope | ErrorKind::Cancelled | ErrorKind::Deadline
        ));
        assert!(page.inspect().is_err());
        assert!(owner.diagnostic_signaling.mailbox.is_none());
        assert!(owner.diagnostic_signaling.response.upgrade().is_some());
        assert_eq!(server.count(), 1);
        drop(page);
        assert!(owner.diagnostic_signaling.response.upgrade().is_none());
    }
}

#[tokio::test]
async fn eight_retired_page_reservations_block_ninth_slot_without_http_and_release_in_any_order() {
    let server = Server::new(vec![json!([]); 8]).await;
    let rt = runtime();
    let mut pages = Vec::new();
    for account in 1..=8 {
        let ctx = context(&rt, account);
        let mut owner = owner(&server.origin, rt.clone(), account);
        let call = admit(&mut owner, &ctx, 1);
        pages.push(owner.diagnostic_poll(&ctx, call).await.unwrap());
        drop(owner);
    }
    let probe = owner(&server.origin, rt, 9);
    assert!(matches!(probe.transport.diagnostic_response_slot(),Err(e) if e.kind==ErrorKind::Busy));
    assert_eq!(server.count(), 8);
    drop(pages.swap_remove(3));
    let slot = probe.transport.diagnostic_response_slot().unwrap();
    assert!(matches!(probe.transport.diagnostic_response_slot(),Err(e) if e.kind==ErrorKind::Busy));
    drop(slot);
    drop(pages);
    let mut slots = Vec::new();
    for _ in 0..8 {
        slots.push(probe.transport.diagnostic_response_slot().unwrap());
    }
}

#[tokio::test]
async fn answerer_requires_selected_session_first_offer_then_fixed_sid_and_single_identity_handle()
{
    let server = Server::new(vec![
        json!([desc("offer"), ice()]),
        json!([event(
            "offer",
            Some("rollover"),
            json!({"type":"offer","sdp":"v=0\r\n"})
        )]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    owner.diagnostic_signaling.selected =
        Some(DiagnosticPeer::new(call, "peer", "generated-unused", false).unwrap());
    owner.diagnostic_signaling.awaiting_offer = true;
    let first = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let peer = owner.diagnostic_take_selected_peer(&ctx, call).unwrap();
    assert!(
        first
            .inspect()
            .unwrap()
            .signals
            .iter()
            .all(|s| s.peer().same_negotiation(&peer))
    );
    assert!(owner.diagnostic_take_selected_peer(&ctx, call).is_err());
    drop(first);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::WrongScope);
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn whole_response_byte_charge_intersects_count_caps_and_failure_releases() {
    // Legal per-string lengths and JSON node/body limits; aggregate typed text
    // exceeds192KiB before the last copy. Two descriptions alone remain valid.
    let sdp = format!(
        "v=0\r\n{}",
        format!("a={}\r\n", "x".repeat(2000)).repeat(32)
    );
    let candidate = format!("candidate:{}", "x".repeat(2038));
    let ice_big = event(
        "candidate",
        Some("sid"),
        json!({"candidate":{"candidate":candidate,"sdpMid":"0","sdpMLineIndex":0}}),
    );
    let mut rows = vec![event("answer", Some("sid"), json!({"type":"answer","sdp":sdp})); 2];
    rows.extend(vec![ice_big; 40]);
    let server = Server::new(vec![json!(rows)]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::TooLarge);
    assert!(owner.diagnostic_signaling.response.upgrade().is_none());
    println!(
        "mailbox layouts: signal={} lease={} mailbox={} page={} fixed-slot-upper={} typed-limit={}",
        std::mem::size_of::<ReceivedSignal>(),
        std::mem::size_of::<ResponseLease>(),
        std::mem::size_of::<Mailbox>(),
        std::mem::size_of::<DiagnosticPoll>(),
        (66 + 3 * 32) * std::mem::size_of::<ReceivedSignal>(),
        TYPED_BYTES
    );
}

#[tokio::test]
async fn near_ceiling_payload_moves_through_three_pages_with_one_response_charge() {
    let sdp = format!(
        "v=0\r\n{}",
        format!("a={}\r\n", "x".repeat(2000)).repeat(32)
    );
    let candidate = format!("candidate:{}", "x".repeat(1040));
    let mut rows = vec![event("answer", Some("sid"), json!({"type":"answer","sdp":sdp})); 2];
    rows.extend(vec![
        event(
            "candidate",
            Some("sid"),
            json!({"candidate":{
                "candidate":candidate, "sdpMid":"0", "sdpMLineIndex":0
            }})
        );
        64
    ]);
    let raw_bytes = serde_json::to_vec(&rows).unwrap().len();
    let server = Server::new(vec![json!(rows)]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    let first = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let retained = first.inspect().unwrap().retained_typed_bytes;
    assert!((190 * 1024..=TYPED_BYTES).contains(&retained));
    let pending_capacity = owner
        .diagnostic_signaling
        .mailbox
        .as_ref()
        .unwrap()
        .signals
        .capacity();
    let second = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let third = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let pages = [first, second, third];
    assert_eq!(
        pages
            .iter()
            .map(|p| p.inspect().unwrap().signals.len())
            .collect::<Vec<_>>(),
        [32, 32, 2]
    );
    // Every row shares the selected identity; count its owned text once.
    // Per-row Arc pointers remain part of the separately measured fixed slots.
    let identity = &pages[0].signals[0].peer.0;
    let mut text_bytes = identity.session.len() + identity.negotiation.as_str().len();
    for page in &pages {
        assert_eq!(page.inspect().unwrap().retained_typed_bytes, retained);
        for received in &page.signals {
            assert!(Arc::ptr_eq(identity, &received.peer.0));
            text_bytes += match &received.signal {
                RetainedSignal::Offer(s) | RetainedSignal::Answer(s) => s.len(),
                RetainedSignal::Candidate { candidate, mid, .. } => candidate.len() + mid.len(),
            };
        }
    }
    assert_eq!(text_bytes, retained);
    assert!(!pages[2].inspect().unwrap().has_more);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Busy);
    assert_eq!(server.count(), 1);
    println!(
        "mailbox peak case: raw-array-bytes={raw_bytes} typed-text={retained} pending-capacity={pending_capacity} returned-slots=66 signal-layout={} lease-layout={} page-layout={}; JSON tree/allocator/HTTP/native overhead additional",
        std::mem::size_of::<ReceivedSignal>(),
        std::mem::size_of::<ResponseLease>(),
        std::mem::size_of::<DiagnosticPoll>()
    );
    drop(pages);
    assert!(owner.diagnostic_signaling.response.upgrade().is_none());
    assert_eq!(owner.diagnostic_signaling.candidates, 64);
    assert_eq!(owner.diagnostic_signaling.descriptions, 2);
}

#[tokio::test]
async fn dropped_destructive_future_does_not_allow_another_poll() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    {
        let polling = owner.diagnostic_poll(&ctx, call);
        tokio::pin!(polling);
        tokio::select! { _=&mut polling=>panic!("response cannot complete"), _=tokio::time::sleep(Duration::from_millis(10))=>{} }
    }
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::Uncertain);
    assert!(owner.diagnostic_signaling.mailbox.is_none());
}

#[test]
fn original_saturating_counts_remain_lower_bounds_at_forty_rows() {
    let mut rows = vec![json!({"type":"message","data":"unparsed"}); 40];
    rows[0] = json!({"type":"usersInRoom"});
    rows[1] = json!({"type":"other"});
    rows[2] = json!({});
    let counts = DiagnosticRowCounts::observe(&rows);
    assert_eq!(
        (
            counts.rows_at_least,
            counts.rows_exact,
            counts.sampled_rows,
            counts.messages,
            counts.users_in_room,
            counts.other_type,
            counts.missing_type,
            counts.types_truncated
        ),
        (34, false, 33, 30, 1, 1, 1, true)
    );
}

#[tokio::test]
async fn authoritative_membership_without_selection_is_none_and_signals_cannot_select() {
    let server = Server::new(vec![
        json!([snapshot(vec![member("peer")])]),
        json!([desc("offer")]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut owner = owner(&server.origin, rt, 1);
    let call = admit(&mut owner, &ctx, 1);
    owner.diagnostic_signaling.selected = None;
    let page = owner.diagnostic_poll(&ctx, call).await.unwrap();
    let view = page.inspect().unwrap();
    assert!(view.membership_observed);
    assert_eq!(view.selected_peer_present, None);
    assert_eq!(view.peer_count, 1);
    assert!(view.signals.is_empty());
    drop(page);
    assert_eq!(failed(&mut owner, &ctx, call).await, ErrorKind::WrongScope);
    assert_eq!(server.count(), 2);
}
