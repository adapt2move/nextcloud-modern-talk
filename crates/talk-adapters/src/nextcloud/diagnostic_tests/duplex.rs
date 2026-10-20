//! Consumer lifecycle tests on actual bounded HTTP, distinct from transport tests.
use super::mailbox_tests::{Server, admit, context, desc, event, ice, owner, runtime};
use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

fn outgoing() -> Signal {
    Signal::Candidate {
        candidate: Text::new("candidate:1 1 udp 1 127.0.0.1 9000 typ host").unwrap(),
        mid: Text::new("0").unwrap(),
        index: 0,
    }
}
async fn head(socket: &mut TcpStream) {
    let mut request = Vec::new();
    loop {
        let mut buf = [0; 1024];
        let n = socket.read(&mut buf).await.unwrap();
        assert!(n > 0 && request.len() + n <= 8192);
        request.extend_from_slice(&buf[..n]);
        if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
            let text = std::str::from_utf8(&request[..end]).unwrap();
            let length = text
                .lines()
                .find_map(|l| {
                    l.to_lowercase()
                        .strip_prefix("content-length: ")
                        .map(|n| n.parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            assert!(length <= 4096);
            if request.len() >= end + 4 + length {
                return;
            }
        }
    }
}
async fn reply(socket: &mut TcpStream, data: Value) {
    let body = json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":data}}).to_string();
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",body.len()).as_bytes()).await.unwrap();
    socket.write_all(body.as_bytes()).await.unwrap();
}
#[tokio::test]
async fn outbound_progress_while_destructive_get_waits_and_guarded_pages_block_next_http() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut read, _) = listener.accept().await.unwrap();
        head(&mut read).await;
        seen_tx.send(()).unwrap();
        let (mut write, _) = listener.accept().await.unwrap();
        head(&mut write).await;
        reply(&mut write, Value::Null).await;
        release_rx.await.unwrap();
        reply(&mut read, json!(vec![ice(); 40])).await;
    });
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&origin, rt, 1);
    let call = admit(&mut client, &ctx, 1);
    let peer = client.diagnostic_take_selected_peer(&ctx, call).unwrap();
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    let read = tokio::spawn(client.diagnostic_prepare_read(&ctx, call).unwrap().wait());
    tokio::time::timeout(Duration::from_secs(1), seen_rx)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(client.diagnostic_prepare_read(&ctx,call),Err(e) if e.kind==ErrorKind::Busy));
    let write = client
        .diagnostic_prepare_send(&ctx, &peer, outgoing())
        .unwrap();
    assert!(
        matches!(client.diagnostic_prepare_send(&ctx,&peer,outgoing()),Err(e) if e.kind==ErrorKind::Busy)
    );
    let done = tokio::time::timeout(Duration::from_secs(1), write.wait())
        .await
        .unwrap();
    assert_eq!(
        client.diagnostic_finish_write(&ctx, call, done).unwrap(),
        DiagnosticWriteResult::AcceptedForProcessing
    );
    assert!(!read.is_finished());
    assert_eq!(client.diagnostic_signaling.sent_candidates, 1);
    release_tx.send(()).unwrap();
    let first = client
        .diagnostic_finish_read(&ctx, call, read.await.unwrap())
        .unwrap();
    assert_eq!(first.inspect().unwrap().signals.len(), 32);
    let last = client.diagnostic_drain_page(&ctx, call).unwrap();
    assert!(!last.inspect().unwrap().has_more);
    drop(first);
    assert!(matches!(client.diagnostic_prepare_read(&ctx,call),Err(e) if e.kind==ErrorKind::Busy));
    client.diagnostic_stop_signaling();
    assert!(last.inspect().is_err());
    assert!(
        matches!(client.diagnostic_prepare_send(&ctx,&peer,outgoing()),Err(e) if e.kind==ErrorKind::Conflict)
    );
    drop(last);
    server.await.unwrap();
}
#[tokio::test]
async fn dropped_ticket_future_or_completion_is_terminal_without_a_second_get() {
    for phase in 0..3 {
        let server = Server::new(vec![json!([])]).await;
        let rt = runtime();
        let ctx = context(&rt, 1);
        let mut client = owner(&server.origin, rt, 1);
        let call = admit(&mut client, &ctx, 1);
        client.diagnostic_start_duplex(&ctx, call).unwrap();
        let read = client.diagnostic_prepare_read(&ctx, call).unwrap();
        match phase {
            0 => drop(read),
            1 => drop(Box::pin(read.wait())),
            _ => drop(read.wait().await),
        }
        assert!(
            matches!(client.diagnostic_prepare_read(&ctx,call),Err(e) if e.kind==ErrorKind::Conflict)
        );
        assert_eq!(server.count(), if phase == 2 { 1 } else { 0 });
        assert!(client.diagnostic_signaling.response.upgrade().is_none());
    }
}
#[tokio::test]
async fn full_validation_cumulative_duplicate_counts_and_bad_tail_apply_to_split_reads() {
    let mut bad = vec![ice(); 32];
    bad.push(event(
        "answer",
        None,
        json!({"type":"answer","sdp":"v=0\r\n"}),
    ));
    for replies in [
        vec![
            json!(vec![ice(); 40]),
            json!(vec![ice(); 24]),
            json!([ice()]),
        ],
        vec![json!(bad)],
    ] {
        let server = Server::new(replies).await;
        let rt = runtime();
        let ctx = context(&rt, 1);
        let mut client = owner(&server.origin, rt, 1);
        let call = admit(&mut client, &ctx, 1);
        client.diagnostic_start_duplex(&ctx, call).unwrap();
        let mut pages = 0;
        let failure = loop {
            let read = client
                .diagnostic_prepare_read(&ctx, call)
                .unwrap()
                .wait()
                .await;
            match client.diagnostic_finish_read(&ctx, call, read) {
                Err(e) => break e.kind,
                Ok(mut page) => loop {
                    let more = page.inspect().unwrap().has_more;
                    pages += 1;
                    drop(page);
                    if !more {
                        break;
                    }
                    page = client.diagnostic_drain_page(&ctx, call).unwrap();
                },
            }
        };
        assert!(matches!(
            failure,
            ErrorKind::TooLarge | ErrorKind::Malformed
        ));
        assert_eq!(pages, if failure == ErrorKind::TooLarge { 3 } else { 0 });
        assert!(client.diagnostic_signaling.mailbox.is_none());
        assert!(client.diagnostic_signaling.response.upgrade().is_none());
        assert!(client.diagnostic_prepare_read(&ctx, call).is_err());
        assert_eq!(server.count(), if pages == 0 { 1 } else { 3 });
    }
}
#[tokio::test]
async fn stale_selection_scope_deadline_and_cancel_never_commit_or_emit() {
    for fault in 0..4 {
        let server = Server::new(vec![json!([desc("answer")])]).await;
        let rt = runtime();
        let mut ctx = context(&rt, 1);
        let mut client = owner(&server.origin, rt.clone(), 1);
        let call = admit(&mut client, &ctx, 1);
        client.diagnostic_start_duplex(&ctx, call).unwrap();
        let read = client
            .diagnostic_prepare_read(&ctx, call)
            .unwrap()
            .wait()
            .await;
        let mut target = call;
        match fault {
            0 => target.call = talk_core::ids::CallGeneration::from_local(88),
            1 => ctx.cancellation.cancel(),
            2 => ctx.deadline = rt.now(),
            _ => {
                client.diagnostic_signaling.selected =
                    Some(DiagnosticPeer::new(call, "different", "sid", true).unwrap())
            }
        }
        assert!(client.diagnostic_finish_read(&ctx, target, read).is_err());
        assert!(client.diagnostic_signaling.mailbox.is_none());
        assert!(client.diagnostic_signaling.response.upgrade().is_none());
        assert_eq!(server.count(), 1);
        assert!(client.diagnostic_prepare_read(&ctx, call).is_err());
    }
}
#[tokio::test]
async fn stopped_settled_write_returns_lane_for_one_terminal_leave() {
    let server = Server::new(vec![Value::Null, Value::Null]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&server.origin, rt, 1);
    let call = admit(&mut client, &ctx, 1);
    let peer = client.diagnostic_take_selected_peer(&ctx, call).unwrap();
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    let done = client
        .diagnostic_prepare_send(&ctx, &peer, outgoing())
        .unwrap()
        .wait()
        .await;
    client.diagnostic_stop_signaling();
    assert!(client.diagnostic_finish_write(&ctx, call, done).is_err());
    let leave = client
        .diagnostic_prepare_leave(&ctx, call)
        .unwrap()
        .wait()
        .await;
    assert_eq!(
        client.diagnostic_finish_write(&ctx, call, leave).unwrap(),
        DiagnosticWriteResult::Left
    );
    assert!(client.diagnostic_prepare_leave(&ctx, call).is_err());
    assert_eq!(server.count(), 2);
}
#[tokio::test]
async fn retired_pages_keep_response_charge_but_loop_releases_only_after_all_lanes() {
    let server = Server::new(vec![json!([])]).await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&server.origin, rt.clone(), 1);
    let call = admit(&mut client, &ctx, 1);
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    let read = client
        .diagnostic_prepare_read(&ctx, call)
        .unwrap()
        .wait()
        .await;
    let page = client.diagnostic_finish_read(&ctx, call, read).unwrap();
    let lease = client.diagnostic_signaling.response.clone();
    drop(client);
    assert!(page.inspect().is_err());
    assert!(lease.upgrade().is_some());
    let mut replacement = owner(&server.origin, rt.clone(), 1);
    let new = admit(&mut replacement, &ctx, 2);
    replacement.diagnostic_start_duplex(&ctx, new).unwrap();
    let mut slots = Vec::new();
    for _ in 0..7 {
        slots.push(replacement.transport.diagnostic_response_slot().unwrap());
    }
    assert!(
        matches!(replacement.transport.diagnostic_response_slot(),Err(e) if e.kind==ErrorKind::Busy)
    );
    drop(page);
    assert!(lease.upgrade().is_none());
    let eighth = replacement.transport.diagnostic_response_slot().unwrap();
    drop(eighth);
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn call_departure_is_explicit_bit_not_presence_or_row_count() {
    use super::mailbox_tests::{member, snapshot};
    let mut left = member("peer");
    left["inCall"] = json!(0);
    let server = Server::new(vec![
        json!([snapshot(vec![member("peer")])]),
        json!([snapshot(vec![left])]),
        json!([]),
        json!([snapshot(vec![])]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&server.origin, rt, 1);
    let call = admit(&mut client, &ctx, 1);
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    for (present, in_call) in [
        (Some(true), Some(true)),
        (Some(true), Some(false)),
        (None, None),
        (Some(false), None),
    ] {
        let read = client
            .diagnostic_prepare_read(&ctx, call)
            .unwrap()
            .wait()
            .await;
        let page = client.diagnostic_finish_read(&ctx, call, read).unwrap();
        let view = page.inspect().unwrap();
        assert_eq!(view.selected_peer_present, present);
        assert_eq!(view.selected_peer_in_call, in_call);
        drop(page);
    }
}

#[tokio::test]
async fn missing_unknown_or_invalid_call_metadata_never_fabricates_departure() {
    use super::mailbox_tests::{member, snapshot};
    let mut absent = member("peer");
    absent.as_object_mut().unwrap().remove("inCall");
    let mut unknown = member("peer");
    unknown["inCall"] = json!(16);
    let mut unsupported_phone = member("peer");
    unsupported_phone["inCall"] = json!(8);
    let mut null = member("peer");
    null["inCall"] = Value::Null;
    let mut malformed = member("peer");
    malformed["inCall"] = json!("0");
    let server = Server::new(vec![
        json!([snapshot(vec![absent])]),
        json!([snapshot(vec![unknown])]),
        json!([snapshot(vec![unsupported_phone])]),
        json!([snapshot(vec![null])]),
        json!([snapshot(vec![malformed])]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&server.origin, rt, 1);
    let call = admit(&mut client, &ctx, 1);
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    for _ in 0..4 {
        let read = client
            .diagnostic_prepare_read(&ctx, call)
            .unwrap()
            .wait()
            .await;
        let page = client.diagnostic_finish_read(&ctx, call, read).unwrap();
        let view = page.inspect().unwrap();
        assert_eq!(view.selected_peer_present, Some(true));
        assert_eq!(view.selected_peer_in_call, None);
        drop(page);
    }
    let read = client
        .diagnostic_prepare_read(&ctx, call)
        .unwrap()
        .wait()
        .await;
    assert!(
        matches!(client.diagnostic_finish_read(&ctx,call,read),Err(e) if e.kind==ErrorKind::Malformed)
    );
    assert!(client.diagnostic_signaling.mailbox.is_none());
}

#[tokio::test]
async fn selected_call_metadata_is_scoped_last_snapshot_and_whole_response_atomic() {
    use super::mailbox_tests::{member, snapshot};
    let mut other_left = member("other");
    other_left["inCall"] = json!(0);
    let mut selected_left = member("peer");
    selected_left["inCall"] = json!(0);
    let mut unknown = member("peer");
    unknown.as_object_mut().unwrap().remove("inCall");
    let mut wrong_room = selected_left.clone();
    wrong_room["roomId"] = json!(8);
    let server = Server::new(vec![
        json!([snapshot(vec![member("peer"), other_left])]),
        json!([
            snapshot(vec![selected_left.clone()]),
            snapshot(vec![unknown])
        ]),
        json!([
            snapshot(vec![selected_left]),
            desc("answer"),
            snapshot(vec![wrong_room])
        ]),
    ])
    .await;
    let rt = runtime();
    let ctx = context(&rt, 1);
    let mut client = owner(&server.origin, rt, 1);
    let call = admit(&mut client, &ctx, 1);
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    for expected in [Some(true), None] {
        let completion = client
            .diagnostic_prepare_read(&ctx, call)
            .unwrap()
            .wait()
            .await;
        let page = client
            .diagnostic_finish_read(&ctx, call, completion)
            .unwrap();
        assert_eq!(page.inspect().unwrap().selected_peer_in_call, expected);
        assert_eq!(page.inspect().unwrap().selected_peer_present, Some(true));
        drop(page);
    }
    let completion = client
        .diagnostic_prepare_read(&ctx, call)
        .unwrap()
        .wait()
        .await;
    assert!(
        matches!(client.diagnostic_finish_read(&ctx, call, completion),
        Err(e) if e.kind == ErrorKind::WrongScope)
    );
    assert!(client.diagnostic_signaling.mailbox.is_none());
    assert!(client.diagnostic_signaling.response.upgrade().is_none());
    assert_eq!(client.diagnostic_signaling.descriptions, 0);
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn retired_inflight_ticket_retains_original_account_until_completion_drop() {
    use crate::nextcloud::transport::{AuthenticatedBinding, ServerOrigin};
    use talk_core::ids::HostId;
    use zeroize::Zeroizing;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let (seen_tx, seen_rx) = oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        head(&mut socket).await;
        seen_tx.send(()).unwrap();
        // No response. Client retirement may cancel locally, but a returned
        // opaque completion still owns the original account/loop reservation.
        let mut byte = [0];
        let _ = socket.read(&mut byte).await;
    });
    let rt = runtime();
    let mut ctx = context(&rt, 1);
    ctx.deadline.millis += 2500;
    let mut client = owner(&origin, rt.clone(), 1);
    let call = admit(&mut client, &ctx, 1);
    client.diagnostic_start_duplex(&ctx, call).unwrap();
    let read = tokio::spawn(client.diagnostic_prepare_read(&ctx, call).unwrap().wait());
    tokio::time::timeout(Duration::from_secs(1), seen_rx)
        .await
        .unwrap()
        .unwrap();
    drop(client);
    let replacement = || {
        NextcloudClient::from_identity(
            AuthenticatedBinding {
                scope: ctx.scope,
                host: HostId::from_local(3),
                origin: ServerOrigin::loopback(&origin).unwrap(),
                login_name: Zeroizing::new("synthetic".into()),
                user: Zeroizing::new("synthetic".into()),
                password: Zeroizing::new("unit-only".into()),
                cancellation: Cancellation::default(),
            },
            rt.clone(),
        )
    };
    assert!(matches!(replacement(), Err(e) if e.kind == ErrorKind::Conflict));
    let completion = tokio::time::timeout(Duration::from_secs(6), read)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(replacement(), Err(e) if e.kind == ErrorKind::Conflict));
    drop(completion);
    let replacement = replacement().unwrap();
    assert!(
        matches!(replacement.transport.diagnostic_call_admission(&ctx, call),
        Err(e) if e.kind == ErrorKind::Conflict)
    );
    server.await.unwrap();
}
