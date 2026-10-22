use super::*;
use crate::scope::{CancellationSignal, ClockId};
use crate::tests::{Server, client, context, reply, scope};
use crate::{AppCredential, Request};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

#[tokio::test]
async fn first_dav_upload_with_stored_body_and_lost_headers_is_uncertain() {
    const CONTENT: &[u8] = b"stored before response loss";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let stored = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        loop {
            let mut block = [0; 512];
            let n = socket.read(&mut block).await.unwrap();
            assert_ne!(n, 0, "the complete PUT must reach the server");
            request.extend_from_slice(&block[..n]);
            assert!(request.len() <= 4096);
            if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                let headers = String::from_utf8(request[..end].to_vec())
                    .unwrap()
                    .to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse::<usize>()
                    .unwrap();
                assert_eq!(length, CONTENT.len());
                if request.len() >= end + 4 + length {
                    // Store the complete create-only PUT, then close without headers.
                    return (headers, request[end + 4..].to_vec());
                }
            }
        }
    });
    let runtime = NetworkRuntime::new(ClockId::from_key(1));
    let mut owner = crate::Client::new(
        AppCredential::new(
            ServerOrigin::loopback(&origin).unwrap(),
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
    assert!(!owner.transport.request_dispatched());
    let error = owner
        .upload_file(
            &context(&runtime),
            crate::files::FilePath::parse("/Talk/file.txt").unwrap(),
            std::io::Cursor::new(CONTENT.to_vec()),
            CONTENT.len() as u64,
        )
        .await
        .unwrap_err();
    let (headers, body) = tokio::time::timeout(Duration::from_secs(2), stored)
        .await
        .unwrap()
        .unwrap();
    assert!(headers.starts_with("put /remote.php/dav/files/synthetic-user/talk/file.txt "));
    assert!(headers.lines().any(|line| line == "if-none-match: *"));
    assert_eq!(body, CONTENT);
    assert!(owner.transport.request_dispatched());
    assert_eq!(error.kind, ErrorKind::Uncertain);
    assert_eq!(error.retry, Retry::AfterReconciliation);
    assert_eq!(runtime.0.requests.available_permits(), 8);
}

async fn after_ocs_dispatch() -> (Server, crate::Client, NetworkRuntime) {
    let server = Server::start(vec![reply(json!({
        "id": "synthetic-user", "display-name": "Fixture"
    }))])
    .await;
    let (mut owner, runtime) = client(&server);
    owner
        .request(&context(&runtime), Request::Account)
        .await
        .unwrap();
    assert_eq!(server.count(), 1);
    assert!(owner.transport.request_dispatched());
    assert_eq!(runtime.0.requests.available_permits(), 8);
    (server, owner, runtime)
}

#[tokio::test]
async fn dav_upload_admission_failure_after_ocs_preserves_busy() {
    let (server, mut owner, runtime) = after_ocs_dispatch().await;
    let permits = runtime
        .0
        .requests
        .clone()
        .try_acquire_many_owned(8)
        .unwrap();
    let error = owner
        .upload_file(
            &context(&runtime),
            crate::files::FilePath::parse("/Talk/file.txt").unwrap(),
            std::io::Cursor::new(b"content".to_vec()),
            7,
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Busy);
    assert_eq!(error.retry, Retry::Never);
    assert!(!owner.transport.request_dispatched());
    assert_eq!(server.count(), 1);
    assert_eq!(runtime.0.requests.available_permits(), 0);
    drop(permits);
    assert_eq!(runtime.0.requests.available_permits(), 8);
}

// Make cancellation become visible only after DAV has acquired its HTTP permit.
// This reaches the select boundary instead of failing the entry validation.
struct CancelAfterAdmission {
    runtime: NetworkRuntime,
    cancelled: Arc<AtomicBool>,
}
impl CancellationSignal for CancelAfterAdmission {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    fn is_cancelled(&self) -> bool {
        if self.runtime.0.requests.available_permits() == 7 {
            self.cancel();
        }
        self.cancelled.load(Ordering::Acquire)
    }
}

#[tokio::test]
async fn dav_upload_cancelled_after_admission_before_execute_is_certain() {
    let (server, mut owner, runtime) = after_ocs_dispatch().await;
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut ctx = context(&runtime);
    ctx.cancellation = Cancellation::from_signal(CancelAfterAdmission {
        runtime: runtime.clone(),
        cancelled: cancelled.clone(),
    });
    let error = owner
        .upload_file(
            &ctx,
            crate::files::FilePath::parse("/Talk/file.txt").unwrap(),
            std::io::Cursor::new(b"content".to_vec()),
            7,
        )
        .await
        .unwrap_err();
    assert!(cancelled.load(Ordering::Acquire));
    assert_eq!(error.kind, ErrorKind::Cancelled);
    assert_eq!(error.retry, Retry::Never);
    assert!(!owner.transport.request_dispatched());
    assert_eq!(server.count(), 1);
    assert_eq!(runtime.0.requests.available_permits(), 8);
}
