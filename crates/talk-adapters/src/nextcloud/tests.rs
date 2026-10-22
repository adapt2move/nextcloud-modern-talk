use super::*;
#[path = "alpha_tests.rs"]
mod alpha_tests;
#[path = "content_tests.rs"]
mod content_tests;
#[path = "note_tests.rs"]
mod note_tests;
#[path = "notification_tests.rs"]
mod notification_tests;
#[path = "people_tests.rs"]
mod people_tests;
#[path = "tag_tests.rs"]
mod tag_tests;
use serde_json::json;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use talk_core::ids::{AccountId, AccountScope, ClockEpoch, HostId, RuntimeGeneration};
use talk_core::policy::Moment;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

struct Reply {
    status: u16,
    headers: String,
    body: Vec<u8>,
    delay: Duration,
    chunked: bool,
    echo_reference: bool,
}
impl Reply {
    fn data(value: Value) -> Self {
        Self::envelope(200, "ok", 200, value)
    }
    fn envelope(status: u16, result: &str, code: u16, value: Value) -> Self {
        Self {
            status,
            headers: String::new(),
            body: serde_json::to_vec(
                &json!({"ocs":{"meta":{"status":result,"statuscode":code,"message":""},"data":value}}),
            )
            .unwrap(),
            delay: Duration::ZERO,
            chunked: false,
            echo_reference: false,
        }
    }
}
struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<Vec<u8>>>>,
    job: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.job.abort();
    }
}
impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/cloud/", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        let job = tokio::spawn(async move {
            for mut reply in replies {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 1024];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    assert!(request.len() + n <= 512 * 1024);
                    request.extend_from_slice(&buf[..n]);
                    if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map(|s| s.trim().parse().unwrap())
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                if reply.echo_reference {
                    let raw = String::from_utf8_lossy(&request);
                    let body = raw.split_once("\r\n\r\n").unwrap().1;
                    let reference = body
                        .split('&')
                        .find_map(|field| field.strip_prefix("referenceId="))
                        .unwrap();
                    let mut response: Value = serde_json::from_slice(&reply.body).unwrap();
                    response["ocs"]["data"]["referenceId"] = json!(reference);
                    reply.body = serde_json::to_vec(&response).unwrap();
                }
                observed.lock().unwrap().push(request);
                tokio::time::sleep(reply.delay).await;
                let framing = if reply.chunked {
                    "Transfer-Encoding: chunked".to_owned()
                } else {
                    format!("Content-Length: {}", reply.body.len())
                };
                let mime = if reply.headers.to_ascii_lowercase().contains("content-type:") {
                    ""
                } else {
                    "Content-Type: application/json\r\n"
                };
                let header = format!(
                    "HTTP/1.1 {} Test\r\n{}{}\r\nConnection: close\r\n{}\r\n",
                    reply.status, mime, framing, reply.headers
                );
                if stream.write_all(header.as_bytes()).await.is_ok() {
                    if reply.chunked {
                        for part in reply.body.chunks(8192) {
                            if stream
                                .write_all(format!("{:x}\r\n", part.len()).as_bytes())
                                .await
                                .is_err()
                                || stream.write_all(part).await.is_err()
                                || stream.write_all(b"\r\n").await.is_err()
                            {
                                break;
                            }
                        }
                        let _ = stream.write_all(b"0\r\n\r\n").await;
                    } else {
                        let _ = stream.write_all(&reply.body).await;
                    }
                }
            }
        });
        Self { origin, seen, job }
    }
    fn count(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}
fn scope() -> AccountScope {
    AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(1),
    }
}
fn runtime() -> NetworkRuntime {
    NetworkRuntime::new(ClockEpoch::from_local(1))
}
fn context(runtime: &NetworkRuntime) -> OperationContext {
    let now = runtime.now();
    OperationContext {
        scope: scope(),
        deadline: Moment {
            epoch: now.epoch,
            millis: now.millis + 2900,
        },
        cancellation: Cancellation::default(),
    }
}
fn client(origin: &str, runtime: NetworkRuntime) -> NextcloudClient {
    NextcloudClient::from_identity(
        AuthenticatedBinding {
            scope: scope(),
            host: HostId::from_local(1),
            origin: transport::ServerOrigin::loopback(origin).unwrap(),
            login_name: Zeroizing::new("fixture_user".into()),
            user: Zeroizing::new("fixture_user".into()),
            password: Zeroizing::new("canary-password-not-for-logs".into()),
            cancellation: Cancellation::default(),
        },
        runtime,
    )
    .unwrap()
}
fn caps(major: u16) -> Value {
    json!({"version":{"major":major},"capabilities":{"spreed":{"features":["conversation-v4","conversation-permissions","conversation-presets","read-only-rooms","chat-v2","chat-reference-id","chat-read-marker","signaling-v3"],"config":{"conversations":{"can-create":true},"chat":{"max-length":32000},"call":{"enabled":true,"recording-consent":0,"end-to-end-encryption":false}}}}})
}
fn room() -> Value {
    json!({"id":7,"token":"testroom","type":2,"displayName":"Fixture","participantType":3,"permissions":511,"readOnly":0,"lobbyState":0,"attributes":0,"canLeaveConversation":true,"isSensitive":false})
}
fn bind(client: &mut NextcloudClient) -> ConversationId {
    client.bind_room(&room()).unwrap()
}

#[tokio::test]
async fn favorite_mutation_returns_reconciled_server_data_without_replay() {
    for (major, favorite) in [(34, true), (35, false)] {
        let mut capabilities = caps(major);
        capabilities["capabilities"]["spreed"]["features"]
            .as_array_mut()
            .unwrap()
            .push(json!("favorites"));
        let mut before = room();
        before["isFavorite"] = json!(!favorite);
        let mut after = room();
        after["isFavorite"] = json!(favorite);
        let server = Server::start(vec![
            Reply::data(capabilities),
            Reply::data(before),
            Reply::data(after.clone()),
            Reply::data(after),
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let ProtocolReply::Conversation(room) = client
            .request(
                context(&runtime),
                ProtocolRequest::SetFavorite {
                    conversation,
                    favorite,
                },
            )
            .await
            .unwrap()
        else {
            panic!("room");
        };
        assert_eq!(room.favorite, Some(favorite));
        assert_eq!(
            room.organization.favorites,
            talk_core::protocol::FeatureSupport::Supported
        );
        assert_eq!(client.room_identity(conversation).unwrap(), Some(7));
        assert!(room.navigation_key.is_none()); // only identity ledger supplies persistence keys
        assert_eq!(server.count(), 4);
        let requests = server.seen.lock().unwrap();
        let mutation = String::from_utf8_lossy(&requests[2]);
        assert!(mutation.starts_with(if favorite { "POST " } else { "DELETE " }));
        assert!(mutation.contains("/room/testroom/favorite"));
    }
}

#[tokio::test]
async fn favorite_unknown_or_unsupported_never_dispatches_mutation() {
    for supported in [false, true] {
        let mut capabilities = caps(35);
        if supported {
            capabilities["capabilities"]["spreed"]["features"]
                .as_array_mut()
                .unwrap()
                .push(json!("favorites"));
        }
        let mut replies = vec![Reply::data(capabilities)];
        if supported {
            replies.push(Reply::data(room()));
        } // missing isFavorite is Unknown
        let server = Server::start(replies).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        assert!(
            client
                .request(
                    context(&runtime),
                    ProtocolRequest::SetFavorite {
                        conversation,
                        favorite: true
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(server.count(), if supported { 2 } else { 1 });
        assert!(
            server
                .seen
                .lock()
                .unwrap()
                .iter()
                .all(|request| request.starts_with(b"GET "))
        );
    }
}

#[tokio::test]
async fn probe_does_not_expose_canonical_id_as_a_display_label() {
    let server = Server::start(vec![Reply::data(json!({"id":"fixture_user"}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    assert_eq!(client.probe(&context(&runtime)).await.unwrap().as_str(), "");
}
fn message() -> Value {
    json!({"id":7,"token":"testroom","actorType":"users","actorId":"fixture_user","actorDisplayName":"Test","timestamp":10,"messageType":"comment","message":"Hallo","referenceId":"foreign"})
}

#[tokio::test]
async fn probe_checks_origin_canonical_user_and_request_headers() {
    let server = Server::start(vec![Reply::data(
        json!({"id":"fixture_user","displayname":"Test"}),
    )])
    .await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    assert!(matches!(
        client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap(),
        ProtocolReply::AccountValid(_)
    ));
    let seen = server.seen.lock().unwrap();
    let headers = String::from_utf8_lossy(&seen[0]).to_lowercase();
    assert!(headers.starts_with("get /cloud/ocs/v2.php/cloud/user?format=json "));
    assert!(
        headers.contains("ocs-apirequest: true")
            && headers.contains("accept-encoding: identity")
            && headers.contains("cache-control: no-store")
    );
    assert!(headers.contains("authorization: basic "));
}

#[tokio::test]
async fn non_ocs_http_failures_preserve_account_admission_and_retry_guidance() {
    for (status, expected) in [
        (403, ErrorKind::ServerAccessDenied),
        (429, ErrorKind::RateLimited),
        (500, ErrorKind::ServerUnavailable),
        (503, ErrorKind::ServerUnavailable),
    ] {
        let mut failed = Reply::data(json!({}));
        failed.status = status;
        failed.headers = "Content-Type: text/html\r\nRetry-After: 9999\r\n".into();
        failed.body = if status == 429 {
            vec![]
        } else {
            b"<html>proxy failure</html>".to_vec()
        };
        let server = Server::start(vec![failed, Reply::data(json!({"id":"fixture_user"}))]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let error = client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected);
        if status == 429 {
            assert_eq!(
                error.retry,
                talk_core::ports::Retry::AfterDelay { millis: 300_000 }
            );
        }
        assert!(!client.cancellation().is_cancelled());
        assert!(matches!(
            client
                .request(context(&runtime), ProtocolRequest::ProbeAccount)
                .await,
            Ok(ProtocolReply::AccountValid(_))
        ));
        assert_eq!(server.count(), 2, "no implicit replay");
    }
}

#[tokio::test]
async fn malformed_cookie_batch_cannot_change_the_next_request_session() {
    let mut failed = Reply::data(json!({"id":"fixture_user"}));
    failed.headers = "Set-Cookie: session=uncommitted; Path=/cloud/\r\nSet-Cookie: other=invalid; Domain=example.invalid; Path=/cloud/\r\n".into();
    let server = Server::start(vec![failed, Reply::data(json!({"id":"fixture_user"}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    assert!(
        client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .is_err()
    );
    client
        .request(context(&runtime), ProtocolRequest::ProbeAccount)
        .await
        .unwrap();
    let seen = server.seen.lock().unwrap();
    assert!(
        !String::from_utf8_lossy(&seen[1])
            .to_ascii_lowercase()
            .contains("cookie:")
    );
}

#[tokio::test]
async fn autonomous_account_probe_reports_only_authoritative_rejection() {
    use talk_core::ids::{CallGeneration, CallScope};
    for (probe, expected, rejected) in [
        (
            Reply::envelope(401, "failure", 997, json!([])),
            nextcloud_talk_api::ErrorKind::ReauthenticationRequired,
            true,
        ),
        (
            Reply::data(json!({"id":"another-user"})),
            nextcloud_talk_api::ErrorKind::WrongScope,
            true,
        ),
        (
            Reply::envelope(403, "failure", 403, json!([])),
            nextcloud_talk_api::ErrorKind::PermissionDenied,
            false,
        ),
        (
            Reply::envelope(500, "failure", 500, json!([])),
            nextcloud_talk_api::ErrorKind::ServerUnavailable,
            false,
        ),
    ] {
        let server = Server::start(vec![
            Reply::data(room()),
            Reply::data(json!({"sessionId":"own-session"})),
            Reply::data(json!([])),
            probe,
            Reply::data(json!({"id":"fixture_user"})),
        ])
        .await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let call = CallScope {
            account: scope(),
            conversation,
            call: CallGeneration::from_local(1),
        };
        let room = nextcloud_talk_api::models::RoomToken::parse("testroom").unwrap();
        client
            .transport
            .api_request(
                &context(&runtime),
                nextcloud_talk_api::Request::Room(room.clone()),
            )
            .await
            .unwrap();
        client
            .transport
            .api_request(
                &context(&runtime),
                nextcloud_talk_api::Request::ActivateRoom(room.clone()),
            )
            .await
            .unwrap();
        let (_reader, mut writer, _stop) = client
            .transport
            .begin_meeting(
                &context(&runtime),
                room,
                nextcloud_talk_api::models::ParticipantFlags::new(false, false),
                false,
                &nextcloud_talk_api::models::SignalingSettings {
                    mode: nextcloud_talk_api::models::SignalingMode::Internal,
                    ice_servers: vec![],
                    external: None,
                },
            )
            .await
            .unwrap();
        let reports = Arc::new(Mutex::new(Vec::new()));
        let recorded = reports.clone();
        let handler: super::AccountRejection =
            Arc::new(move |scope| recorded.lock().unwrap().push(scope));
        let cancel = client.cancellation();
        let wrong_scope = CallScope {
            account: AccountScope {
                runtime: RuntimeGeneration::from_local(2),
                ..call.account
            },
            ..call
        };
        let wrong_scope_result = super::meeting_runtime::refresh(
            &mut writer,
            wrong_scope,
            &runtime,
            &cancel,
            Some(&handler),
        )
        .await;
        assert!(matches!(
            wrong_scope_result,
            Err(nextcloud_talk_api::ErrorKind::WrongScope)
        ));
        assert!(
            !cancel.is_cancelled(),
            "request scope mismatch is not account identity loss"
        );
        assert!(reports.lock().unwrap().is_empty());
        assert_eq!(
            server.count(),
            3,
            "wrong scope never dispatches the account probe"
        );
        let result =
            super::meeting_runtime::refresh(&mut writer, call, &runtime, &cancel, Some(&handler))
                .await;
        assert!(matches!(result, Err(kind) if kind == expected));
        assert_eq!(cancel.is_cancelled(), rejected);
        assert_eq!(
            *reports.lock().unwrap(),
            if rejected { vec![scope()] } else { vec![] }
        );
        if !rejected {
            assert!(matches!(
                client
                    .request(context(&runtime), ProtocolRequest::ProbeAccount)
                    .await,
                Ok(ProtocolReply::AccountValid(_))
            ));
        }
    }
}

#[tokio::test]
async fn errors_are_redacted_and_scope_specific() {
    for (status, code, expected) in [
        (401, 997, ErrorKind::ReauthenticationRequired),
        (403, 403, ErrorKind::PermissionDenied),
        (404, 404, ErrorKind::NotFound),
        (429, 429, ErrorKind::RateLimited),
        (503, 503, ErrorKind::ServerUnavailable),
    ] {
        let mut reply = Reply::envelope(
            status,
            "failure",
            code,
            json!({"private":"canary-password-not-for-logs"}),
        );
        reply.headers = "Retry-After: 5\r\n".into();
        let server = Server::start(vec![reply]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let error = client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err();
        assert_eq!(error.kind, expected);
        assert!(!format!("{error:?}").contains("canary"));
        if status == 429 {
            assert_eq!(
                error.retry,
                talk_core::ports::Retry::AfterDelay { millis: 5000 }
            );
        }
    }
    let server = Server::start(vec![Reply::data(json!({"id":"someone_else"}))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    assert_eq!(
        client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
}

#[tokio::test]
async fn redirect_compression_bad_structure_and_oversize_fail_before_use() {
    let sink = Server::start(vec![Reply::data(json!({}))]).await;
    let mut redirect = Reply::data(json!({}));
    redirect.status = 302;
    redirect.headers = format!("Location: {}\r\n", sink.origin);
    let mut compressed = Reply::data(json!({}));
    compressed.headers = "Content-Encoding: gzip\r\n".into();
    let mut bad = Reply::data(json!({}));
    bad.body = br#"{"ocs":{"meta":true,"meta":false}}"#.to_vec();
    let mut large = Reply::data(json!({}));
    large.body = vec![b' '; nextcloud_talk_api::MAX_RESPONSE_BYTES + 1];
    let mut chunked = Reply::data(json!({}));
    chunked.body = vec![b' '; nextcloud_talk_api::MAX_RESPONSE_BYTES + 1];
    chunked.chunked = true;
    for (reply, kind) in [
        (redirect, ErrorKind::RedirectRejected),
        (compressed, ErrorKind::Unsupported),
        (bad, ErrorKind::Malformed),
        (large, ErrorKind::TooLarge),
        (chunked, ErrorKind::TooLarge),
    ] {
        let server = Server::start(vec![reply]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        assert_eq!(
            client
                .request(context(&runtime), ProtocolRequest::ProbeAccount)
                .await
                .unwrap_err()
                .kind,
            kind
        );
    }
    assert_eq!(sink.count(), 0);
}

#[tokio::test]
async fn identity_cancellation_binding_lifetime_and_observer_drop_keep_admission_closed() {
    let mut delayed = Reply::data(json!({"id":"fixture_user"}));
    delayed.delay = Duration::from_millis(250);
    let server = Server::start(vec![delayed]).await;
    let runtime = runtime();
    let identity_cancel = Cancellation::default();
    let binding = |generation, cancel| AuthenticatedBinding {
        scope: AccountScope {
            runtime: RuntimeGeneration::from_local(generation),
            ..scope()
        },
        host: HostId::from_local(1),
        origin: ServerOrigin::loopback(&server.origin).unwrap(),
        login_name: Zeroizing::new("fixture_user".into()),
        user: Zeroizing::new("fixture_user".into()),
        password: Zeroizing::new("private".into()),
        cancellation: cancel,
    };
    let mut owner =
        NextcloudClient::from_identity(binding(1, identity_cancel.clone()), runtime.clone())
            .unwrap();
    assert!(matches!(
        NextcloudClient::from_identity(binding(2, Cancellation::default()), runtime.clone()),
        Err(PortError {
            kind: ErrorKind::Conflict,
            ..
        })
    ));
    assert!(
        tokio::time::timeout(
            Duration::from_millis(40),
            owner.request(context(&runtime), ProtocolRequest::ProbeAccount)
        )
        .await
        .is_err()
    );
    // The observer ended; the account owner and generation reservation did not.
    assert!(!identity_cancel.is_cancelled());
    assert!(matches!(
        NextcloudClient::from_identity(binding(2, Cancellation::default()), runtime.clone()),
        Err(PortError {
            kind: ErrorKind::Conflict,
            ..
        })
    ));
    identity_cancel.cancel();
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(server.count(), 1);
    drop(owner);
    assert!(matches!(
        NextcloudClient::from_identity(binding(2, identity_cancel), runtime.clone()),
        Err(PortError {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    // Only explicit owner drop releases the reservation. Identity must supply a
    // separately validated new generation and never recycle the cancelled token.
    let mut fresh =
        NextcloudClient::from_identity(binding(2, Cancellation::default()), runtime).unwrap();
    let old = OperationContext {
        scope: scope(),
        deadline: Moment {
            epoch: ClockEpoch::from_local(1),
            millis: 1000,
        },
        cancellation: Cancellation::default(),
    };
    assert_eq!(
        fresh
            .request(old, ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn confirmed_rejection_cancels_shared_identity_and_late_work() {
    let server = Server::start(vec![Reply::envelope(401, "failure", 997, json!({}))]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let identity = owner.cancellation();
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::ReauthenticationRequired
    );
    assert!(identity.is_cancelled());
    assert_eq!(owner.availability(), Availability::Unconfigured);
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::ListConversations)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn deadline_during_response_and_cancelled_completion_cannot_succeed() {
    let mut delayed = Reply::data(json!({"id":"fixture_user"}));
    delayed.delay = Duration::from_millis(100);
    let server = Server::start(vec![delayed]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let mut request = context(&runtime);
    request.deadline.millis = runtime.now().millis + 30;
    assert_eq!(
        owner
            .request(request, ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(server.count(), 1); // no retry or detached success delivery
}

#[tokio::test]
async fn cancellation_deadline_and_stale_generation_never_deliver_success() {
    let mut delayed = Reply::data(json!({"id":"fixture_user"}));
    delayed.delay = Duration::from_millis(200);
    let server = Server::start(vec![delayed]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let mut stale = context(&runtime);
    stale.scope.runtime = RuntimeGeneration::from_local(2);
    assert_eq!(
        client
            .request(stale, ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(server.count(), 0);
    let mut expired = context(&runtime);
    expired.deadline.millis = 0;
    assert_eq!(
        client
            .request(expired, ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Deadline
    );
    let request = context(&runtime);
    let cancel = request.cancellation.clone();
    let cancellation = async {
        tokio::time::sleep(Duration::from_millis(35)).await;
        cancel.cancel();
    };
    let (result, _) = tokio::join!(
        client.request(request, ProtocolRequest::ProbeAccount),
        cancellation
    );
    assert_eq!(result.unwrap_err().kind, ErrorKind::Cancelled);
    assert_eq!(server.count(), 1);
    client.cancellation().cancel();
    assert_eq!(
        client
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
}

#[tokio::test]
async fn bounded_history_and_cross_room_cursor() {
    let mut reply = Reply::data(json!([message()]));
    reply.headers = "X-Chat-Last-Given: 7\r\nX-Chat-Last-Common-Read: 4\r\n".into();
    let mut capabilities = caps(34);
    capabilities["capabilities"]["spreed"]["config"]["chat"]["read-privacy"] = json!(0);
    let server = Server::start(vec![Reply::data(capabilities), Reply::data(room()), reply]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let id = bind(&mut client);
    let ProtocolReply::History(history) = client
        .request(
            context(&runtime),
            ProtocolRequest::History {
                conversation: id,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("wrong response")
    };
    assert_eq!(history.messages.as_slice().len(), 1);
    assert_eq!(history.older.unwrap().sequence, 7);
    assert_eq!(history.common_read.unwrap().sequence, 4);
    let request = String::from_utf8_lossy(&server.seen.lock().unwrap()[2]).to_string();
    assert!(request.contains("setReadMarker=0") && request.contains("limit=50"));
    let foreign = ConversationId {
        host: HostId::from_local(2),
        ..id
    };
    assert_eq!(
        client
            .request(
                context(&runtime),
                ProtocolRequest::ReconcileConversation(foreign)
            )
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn dropped_send_is_retained_uncertain_and_is_not_automatically_replayed() {
    let mut response = Reply::data(message());
    response.delay = Duration::from_millis(300);
    let server = Server::start(vec![Reply::data(caps(35)), Reply::data(room()), response]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    let conversation = bind(&mut client);
    let operation = OperationId::from_local(77);
    let send = SendMessage {
        conversation,
        operation,
        text: Text::new("Hallo").unwrap(),
    };
    assert!(
        tokio::time::timeout(
            Duration::from_millis(80),
            client.request(context(&runtime), ProtocolRequest::Send(send))
        )
        .await
        .is_err()
    );
    assert_eq!(server.count(), 3);
    assert_eq!(
        client
            .request(context(&runtime), ProtocolRequest::SendStatus(operation))
            .await
            .unwrap(),
        ProtocolReply::Send(SendReceipt {
            operation,
            state: SendState::Uncertain
        })
    );
    assert_eq!(server.count(), 3);
}

#[tokio::test]
async fn unknown_and_restrictive_policies_never_send() {
    for (attrs, permissions, read_only, expected) in [
        (32, 511, 0, ErrorKind::Unsupported),
        (0, 383, 0, ErrorKind::PermissionDenied),
        (0, 511, 1, ErrorKind::PermissionDenied),
        (4, 511, 0, ErrorKind::PermissionDenied),
    ] {
        let mut data = room();
        data["attributes"] = json!(attrs);
        data["permissions"] = json!(permissions);
        data["readOnly"] = json!(read_only);
        let server = Server::start(vec![Reply::data(caps(35)), Reply::data(data)]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let result = client
            .request(
                context(&runtime),
                ProtocolRequest::Send(SendMessage {
                    conversation,
                    operation: OperationId::from_local(3),
                    text: Text::new("Must not send").unwrap(),
                }),
            )
            .await;
        assert_eq!(result.unwrap_err().kind, expected);
        assert_eq!(server.count(), 2);
    }
}

#[tokio::test]
async fn future_family_missing_capabilities_and_account_capacity_are_bounded() {
    let server = Server::start(vec![Reply::data(caps(36))]).await;
    let runtime = runtime();
    let mut client = client(&server.origin, runtime.clone());
    assert_eq!(
        client
            .request(context(&runtime), ProtocolRequest::ListConversations)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let mut clients = vec![client];
    for n in 2..=8 {
        clients.push(
            NextcloudClient::from_identity(
                AuthenticatedBinding {
                    scope: AccountScope {
                        account: AccountId::from_local(n),
                        ..scope()
                    },
                    host: HostId::from_local(1),
                    origin: transport::ServerOrigin::loopback(&server.origin).unwrap(),
                    login_name: Zeroizing::new("fixture_user".into()),
                    user: Zeroizing::new("fixture_user".into()),
                    password: Zeroizing::new("private".into()),
                    cancellation: Cancellation::default(),
                },
                runtime.clone(),
            )
            .unwrap(),
        );
    }
    let ninth = NextcloudClient::from_identity(
        AuthenticatedBinding {
            scope: scope(),
            host: HostId::from_local(1),
            origin: transport::ServerOrigin::loopback(&server.origin).unwrap(),
            login_name: Zeroizing::new("fixture_user".into()),
            user: Zeroizing::new("fixture_user".into()),
            password: Zeroizing::new("private".into()),
            cancellation: Cancellation::default(),
        },
        runtime,
    );
    assert!(matches!(
        ninth,
        Err(PortError {
            kind: ErrorKind::Busy,
            ..
        })
    ));
}

#[tokio::test]
async fn history_304_is_exhaustion_only_on_the_history_operation() {
    let mut exhausted = Reply::data(json!({}));
    exhausted.status = 304;
    exhausted.body.clear();
    let server = Server::start(vec![Reply::data(caps(34)), Reply::data(room()), exhausted]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let room = bind(&mut owner);
    let ProtocolReply::History(page) = owner
        .request(
            context(&runtime),
            ProtocolRequest::History {
                conversation: room,
                before: None,
            },
        )
        .await
        .unwrap()
    else {
        panic!("history reply")
    };
    assert!(page.messages.as_slice().is_empty() && page.older.is_none());
    let mut not_modified = Reply::data(json!({}));
    not_modified.status = 304;
    not_modified.body.clear();
    let other = Server::start(vec![not_modified]).await;
    drop(owner);
    let mut owner = client(&other.origin, runtime.clone());
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::RedirectRejected
    );
}

#[tokio::test]
async fn stale_call_generation_cannot_leave_or_join_the_current_session() {
    use talk_core::ids::{CallGeneration, CallScope};
    let server = Server::start(Vec::new()).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let conversation = bind(&mut owner);
    let call = CallScope {
        account: scope(),
        conversation,
        call: CallGeneration::from_local(1),
    };
    owner.active = Some(ActiveConversation {
        id: conversation,
        session: Zeroizing::new("private-session".into()),
    });
    owner.observed_call = Some(call);
    owner.diagnostic_membership_scope = Some((call, 7));
    let stale = CallScope {
        call: CallGeneration::from_local(2),
        ..call
    };
    assert_eq!(
        owner
            .request(context(&runtime), ProtocolRequest::Leave(stale))
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(
        owner
            .diagnostic_join_synthetic_call(&context(&runtime), stale)
            .await
            .unwrap_err()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(server.count(), 0);
    assert_eq!(owner.observed_call, Some(call));
}

#[tokio::test]
async fn browser_participant_events_without_sid_do_not_discard_negotiation() {
    use super::diagnostics::{DiagnosticPollStage, SignalRef};
    use talk_core::ids::CallGeneration;
    // Talk24 browser capture I1-SIG-01: unmute is participant-scoped, with no SID.
    // Strings here are invented fixtures, never recorded browser wire values.
    let event = |kind: &str, sid: Option<&str>, to: &str, payload: Value| {
        let mut inner = json!({"from":"remote-session","to":to,"roomType":"video","type":kind,"payload":payload});
        if let Some(sid) = sid {
            inner["sid"] = json!(sid);
        }
        json!({"type":"message","data":inner.to_string()})
    };
    let recipient = "private-session";
    let answer = event(
        "answer",
        Some("negotiation"),
        recipient,
        json!({"type":"answer","sdp":"v=0\r\n"}),
    );
    let ice = event(
        "candidate",
        Some("negotiation"),
        recipient,
        json!({"candidate":{"candidate":"candidate:1 1 udp 1 127.0.0.1 9000 typ host","sdpMid":"0","sdpMLineIndex":0}}),
    );
    let audio = event("unmute", None, recipient, json!({"name":"audio"}));
    let video = event("unmute", None, recipient, json!({"name":"video"}));
    let missing_sid = event(
        "answer",
        None,
        recipient,
        json!({"type":"answer","sdp":"v=0\r\n"}),
    );
    let wrong_recipient = event("unmute", None, "another-session", json!({"name":"audio"}));
    let malformed_ice = event(
        "candidate",
        Some("negotiation"),
        recipient,
        json!({"candidate":{"candidate":"fixture","sdpMid":"0","sdpMLineIndex":null}}),
    );
    let server = Server::start(vec![
        Reply::data(json!([answer, ice, audio, video])),
        Reply::data(json!([wrong_recipient])),
        Reply::data(json!([missing_sid])),
        Reply::data(json!([malformed_ice])),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let conversation = bind(&mut owner);
    let mut call = CallScope {
        account: scope(),
        conversation,
        call: CallGeneration::from_local(1),
    };
    owner.active = Some(ActiveConversation {
        id: conversation,
        session: Zeroizing::new(recipient.into()),
    });
    owner.observed_call = Some(call);
    owner.diagnostic_membership_scope = Some((call, 7));
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "remote-session",
        "negotiation",
    );
    let poll = owner.diagnostic_poll(&context(&runtime), call).await;
    assert!(
        poll.is_ok(),
        "participant event discarded valid negotiation at {:?}",
        owner.diagnostic_poll_stage()
    );
    let poll = poll.unwrap();
    assert_eq!(poll.inspect().unwrap().signals.len(), 2);
    assert!(matches!(
        poll.inspect().unwrap().signals[0].signal(),
        SignalRef::Answer(_)
    ));
    assert!(matches!(
        poll.inspect().unwrap().signals[1].signal(),
        SignalRef::Candidate { .. }
    ));
    assert!(
        poll.inspect().unwrap().signals[0]
            .peer()
            .same_negotiation(poll.inspect().unwrap().signals[1].peer())
    );
    assert_eq!(owner.diagnostic_poll_stage(), DiagnosticPollStage::Complete);
    drop(poll);
    for (index, (expected, stage)) in [
        (ErrorKind::WrongScope, DiagnosticPollStage::Recipient),
        (ErrorKind::Malformed, DiagnosticPollStage::Negotiation),
        (ErrorKind::Malformed, DiagnosticPollStage::Payload),
    ]
    .into_iter()
    .enumerate()
    {
        call.call = CallGeneration::from_local(index as u128 + 2);
        diagnostics::admit_fixture(
            &mut owner,
            &context(&runtime),
            call,
            "remote-session",
            "negotiation",
        );
        let error = owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .err()
            .unwrap();
        assert_eq!(error.kind, expected);
        assert_eq!(owner.diagnostic_poll_stage(), stage);
    }
    let stale = CallScope {
        call: CallGeneration::from_local(2),
        ..call
    };
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), stale)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(owner.diagnostic_poll_stage(), DiagnosticPollStage::Scope);
    assert_eq!(server.count(), 4);
}

#[tokio::test]
async fn cookies_are_host_only_bounded_and_kept_inside_the_owner() {
    let mut first = Reply::data(json!({"id":"fixture_user"}));
    first.headers = "Set-Cookie: fixture_session=private-cookie; Path=/cloud/; HttpOnly\r\n".into();
    let server = Server::start(vec![first, Reply::data(json!({"id":"fixture_user"}))]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    owner
        .request(context(&runtime), ProtocolRequest::ProbeAccount)
        .await
        .unwrap();
    owner
        .request(context(&runtime), ProtocolRequest::ProbeAccount)
        .await
        .unwrap();
    {
        let seen = server.seen.lock().unwrap();
        assert!(
            !String::from_utf8_lossy(&seen[0])
                .to_lowercase()
                .contains("cookie:")
        );
        assert!(String::from_utf8_lossy(&seen[1]).contains("fixture_session=private-cookie"));
    }
    drop(owner);
    for header in [
        "Set-Cookie: fixture_session=private; Domain=example.org; Path=/cloud/\r\n",
        "Set-Cookie: fixture_session=private; Path=/elsewhere/\r\n",
    ] {
        let mut reply = Reply::data(json!({"id":"fixture_user"}));
        reply.headers = header.into();
        let isolated = Server::start(vec![reply]).await;
        let mut owner = client(&isolated.origin, runtime.clone());
        assert!(
            owner
                .request(context(&runtime), ProtocolRequest::ProbeAccount)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn diagnostic_row_counts_preserve_limits_and_clear_on_stale_scope() {
    use super::diagnostics::DiagnosticPollStage;
    use talk_core::ids::CallGeneration;
    let mut oversized = vec![json!({"type":"message","data":"unparsed"}); 85];
    oversized[0] = json!({"type":"usersInRoom","data":[]});
    oversized[1] = json!({"type":"unknown-never-exported"});
    oversized[2] = json!({"data":"not-an-envelope"});
    let server = Server::start(vec![
        Reply::data(json!(oversized)),
        Reply::data(json!([{"type":"usersInRoom","data":[]}])),
        Reply::data(json!([{"type":"unknown-never-exported"}])),
        Reply::data(json!({"not":"array"})),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let conversation = bind(&mut owner);
    let mut call = CallScope {
        account: scope(),
        conversation,
        call: CallGeneration::from_local(1),
    };
    owner.active = Some(ActiveConversation {
        id: conversation,
        session: Zeroizing::new("synthetic-recipient".into()),
    });
    owner.observed_call = Some(call);
    owner.diagnostic_membership_scope = Some((call, 7));
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "remote-session",
        "negotiation",
    );
    assert!(owner.diagnostic_row_counts().is_none());
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::TooLarge
    );
    assert_eq!(owner.diagnostic_poll_stage(), DiagnosticPollStage::Rows);
    let counts = owner.diagnostic_row_counts().unwrap();
    assert_eq!(
        (counts.rows_at_least, counts.rows_exact, counts.sampled_rows),
        (34, false, 33)
    );
    assert_eq!(
        (
            counts.users_in_room,
            counts.messages,
            counts.other_type,
            counts.missing_type
        ),
        (1, 30, 1, 1)
    );
    assert!(counts.types_truncated);
    assert!(!format!("{counts:?}").contains("unparsed"));
    call.call = CallGeneration::from_local(2);
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "remote-session",
        "negotiation",
    );
    assert!(
        owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .is_ok()
    );
    let counts = owner.diagnostic_row_counts().unwrap();
    assert_eq!(
        (
            counts.rows_at_least,
            counts.rows_exact,
            counts.users_in_room
        ),
        (1, true, 1)
    );
    assert!(!counts.types_truncated);
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(owner.diagnostic_row_counts().unwrap().other_type, 1);
    call.call = CallGeneration::from_local(3);
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "remote-session",
        "negotiation",
    );
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::Malformed
    );
    assert!(owner.diagnostic_row_counts().is_none());
    let stale = CallScope {
        call: CallGeneration::from_local(99),
        ..call
    };
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), stale)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
    assert!(owner.diagnostic_row_counts().is_none());
    assert_eq!(server.count(), 4);
}

fn login_binding(origin: &str, expected: Option<&str>) -> LoginBinding {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).unwrap();
    let password = Zeroizing::new(random.iter().map(|b| format!("{b:02x}")).collect());
    LoginBinding {
        scope: scope(),
        host: HostId::from_local(1),
        origin: ServerOrigin::loopback(origin).unwrap(),
        login_name: Zeroizing::new("Mixed.Case@Example.invalid".into()),
        password,
        expected_user: expected.map(|s| Zeroizing::new(s.into())),
        cancellation: Cancellation::default(),
    }
}

#[tokio::test]
async fn login_alias_is_exact_basic_auth_while_canonical_id_is_qualified_and_slots_release() {
    for major in [34, 35] {
        let server = Server::start(vec![
            Reply::data(json!({"id":"canonical-user"})),
            Reply::data(json!({"id":"canonical-user","displayname":"Synthetic"})),
            Reply::data(caps(major)),
        ])
        .await;
        let runtime = runtime();
        let binding = login_binding(&server.origin, None);
        let request = reqwest::Client::new()
            .get(&server.origin)
            .basic_auth(binding.login_name.as_str(), Some(binding.password.as_str()))
            .build()
            .unwrap();
        let expected = Zeroizing::new(
            request.headers()["Authorization"]
                .to_str()
                .unwrap()
                .to_owned(),
        );
        let (canonical, probe) =
            NextcloudClient::validate_login(binding, runtime.clone(), context(&runtime))
                .await
                .unwrap();
        assert!(canonical.as_str() == "canonical-user");
        assert!(probe.talk_available);
        {
            let seen = server.seen.lock().unwrap();
            assert_eq!(seen.len(), 3);
            for input in seen.iter() {
                assert!(
                    input.split(|b| *b == b'\n').any(|line| line
                        .strip_prefix(b"authorization: ")
                        .is_some_and(|h| h.strip_suffix(b"\r") == Some(expected.as_bytes()))),
                    "exact alias app authentication header"
                );
            }
        }
        // Successful qualification returns no live client and releases the reservation.
        let owner = client(&server.origin, runtime.clone());
        let before = server.count();
        assert!(matches!(
            NextcloudClient::validate_login(
                login_binding(&server.origin, None),
                runtime.clone(),
                context(&runtime)
            )
            .await,
            Err(PortError {
                kind: ErrorKind::Conflict,
                ..
            })
        ));
        assert_eq!(server.count(), before);
        drop(owner);
        assert!(
            NextcloudClient::from_identity(
                AuthenticatedBinding {
                    scope: scope(),
                    host: HostId::from_local(1),
                    origin: ServerOrigin::loopback(&server.origin).unwrap(),
                    login_name: Zeroizing::new("Mixed.Case@Example.invalid".into()),
                    user: canonical,
                    password: login_binding(&server.origin, None).password,
                    cancellation: Cancellation::default(),
                },
                runtime
            )
            .is_ok()
        );
    }
}

#[tokio::test]
async fn login_validation_failures_never_issue_identity_and_release_account_slots() {
    for (reply, expected, kind) in [
        (
            Reply::data(json!({"id":"other-user"})),
            Some("expected-user"),
            ErrorKind::WrongScope,
        ),
        (
            Reply::envelope(401, "failure", 997, json!({})),
            None,
            ErrorKind::ReauthenticationRequired,
        ),
        (Reply::data(json!({"id":""})), None, ErrorKind::Malformed),
    ] {
        let server = Server::start(vec![reply]).await;
        let runtime = runtime();
        assert!(
            matches!(NextcloudClient::validate_login(login_binding(&server.origin, expected), runtime.clone(), context(&runtime)).await, Err(e) if e.kind == kind)
        );
        assert_eq!(server.count(), 1);
        drop(client(&server.origin, runtime));
    }
    let mut delayed = Reply::data(json!({"id":"canonical-user"}));
    delayed.delay = Duration::from_millis(100);
    let server = Server::start(vec![delayed]).await;
    let runtime = runtime();
    let ctx = context(&runtime);
    let cancellation = ctx.cancellation.clone();
    let validation =
        NextcloudClient::validate_login(login_binding(&server.origin, None), runtime.clone(), ctx);
    tokio::pin!(validation);
    tokio::select! {_ = &mut validation => panic!("validation ended before cancellation"), _ = tokio::time::sleep(Duration::from_millis(20)) => {}}
    cancellation.cancel();
    assert!(matches!(
        validation.await,
        Err(PortError {
            kind: ErrorKind::Cancelled,
            ..
        })
    ));
    assert_eq!(server.count(), 1);
    drop(client(&server.origin, runtime));
}

#[tokio::test]
async fn malformed_canonical_ids_share_initial_and_subsequent_probe_validation() {
    for data in [
        json!({}),
        json!({"id":null}),
        json!({"id":false}),
        json!({"id":42}),
        json!({"id":""}),
        json!({"id":"x".repeat(257)}),
        json!({"id":"bad\nidentifier"}),
    ] {
        for followup in [false, true] {
            let mut replies = Vec::new();
            if followup {
                replies.push(Reply::data(json!({"id":"canonical-user"})));
            }
            replies.push(Reply::data(data.clone()));
            let server = Server::start(replies).await;
            let runtime = runtime();
            let binding = login_binding(&server.origin, None);
            assert!(matches!(
                NextcloudClient::validate_login(binding, runtime.clone(), context(&runtime)).await,
                Err(PortError {
                    kind: ErrorKind::Malformed,
                    ..
                })
            ));
            assert_eq!(server.count(), if followup { 2 } else { 1 });
            drop(client(&server.origin, runtime)); // Failed chain released its reservation.
        }
        for request in [
            ProtocolRequest::ProbeAccount,
            ProtocolRequest::InspectAccount,
        ] {
            let server = Server::start(vec![Reply::data(data.clone())]).await;
            let runtime = runtime();
            let mut client = client(&server.origin, runtime.clone());
            assert!(matches!(
                client.request(context(&runtime), request).await,
                Err(PortError {
                    kind: ErrorKind::Malformed,
                    ..
                })
            ));
            assert!(!client.cancellation().is_cancelled());
            assert_eq!(server.count(), 1);
        }
    }
    assert_eq!(
        canonical_id(&json!({"id":"u".repeat(256)})).unwrap().len(),
        256
    );
    assert!(canonical_id(&json!({"id":"valid@example.invalid"})).is_ok());
}

#[tokio::test]
async fn selected_peer_membership_is_scoped_ordered_and_all_or_nothing() {
    use talk_core::ids::CallGeneration;
    let signal = json!({"type":"message","data":json!({"from":"selected-session","to":"local-session","roomType":"video","type":"answer","sid":"old-sid","payload":{"type":"answer","sdp":"v=0\r\n"}}).to_string()});
    let member = |session: &str| json!({"userId":"same-actor", "actorId":"same-actor", "actorType":"users", "sessionId":session, "roomId":7, "lastPing":1, "inCall":7, "participantPermissions":511});
    let snapshot = |users: Value| json!({"type":"usersInRoom","data":users});
    let mut wrong_room = member("selected-session");
    wrong_room["roomId"] = json!(8);
    let mut malformed = member("selected-session");
    malformed["sessionId"] = json!(null);
    let mut missing_room = member("selected-session");
    missing_room.as_object_mut().unwrap().remove("roomId");
    let mut string_room = member("selected-session");
    string_room["roomId"] = json!("7");
    let server = Server::start(vec![
        Reply::data(json!([signal])),
        Reply::data(json!([])),
        Reply::data(json!([snapshot(json!([member("selected-session")]))])),
        Reply::data(json!([snapshot(json!([member(
            "same-actor-other-session"
        )]))])),
        Reply::data(json!([
            snapshot(json!([member("selected-session")])),
            snapshot(json!([]))
        ])),
        Reply::data(json!([
            snapshot(json!([])),
            snapshot(json!([member("selected-session")]))
        ])),
        Reply::data(json!([snapshot(json!([])), snapshot(json!([wrong_room]))])),
        Reply::data(json!([snapshot(json!([malformed]))])),
        Reply::data(json!([snapshot(json!([missing_room]))])),
        Reply::data(json!([snapshot(json!([string_room]))])),
        Reply::data(json!([snapshot(json!([
            member("selected-session"),
            member("selected-session")
        ]))])),
        Reply::data(json!([snapshot(json!([])), {"type":"message","data":"malformed"}])),
        Reply::data(json!([snapshot(json!([]))])),
    ])
    .await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let conversation = bind(&mut owner);
    let mut call = CallScope {
        account: scope(),
        conversation,
        call: CallGeneration::from_local(1),
    };
    owner.active = Some(ActiveConversation {
        id: conversation,
        session: Zeroizing::new("local-session".into()),
    });
    owner.observed_call = Some(call);
    owner.diagnostic_membership_scope = Some((call, 7));
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "selected-session",
        "old-sid",
    );
    let first = owner
        .diagnostic_poll(&context(&runtime), call)
        .await
        .unwrap();
    let mut selected = owner
        .diagnostic_take_selected_peer(&context(&runtime), call)
        .unwrap();
    assert!(!first.inspect().unwrap().membership_observed);
    assert_eq!(first.inspect().unwrap().selected_peer_present, None);
    drop(first);
    let absent = owner
        .diagnostic_poll_selected(&context(&runtime), call, Some(&selected))
        .await
        .unwrap();
    assert!(!absent.inspect().unwrap().membership_observed);
    assert_eq!(absent.inspect().unwrap().selected_peer_present, None);
    drop(absent);
    for (present, count) in [(true, 1), (false, 1), (false, 0), (true, 1)] {
        let observed = owner
            .diagnostic_poll_selected(&context(&runtime), call, Some(&selected))
            .await
            .unwrap();
        assert!(observed.inspect().unwrap().membership_observed);
        assert_eq!(
            observed.inspect().unwrap().selected_peer_present,
            Some(present)
        );
        assert_eq!(observed.inspect().unwrap().peer_count, count);
    }
    for (index, kind) in [
        ErrorKind::WrongScope,
        ErrorKind::Malformed,
        ErrorKind::Malformed,
        ErrorKind::Malformed,
        ErrorKind::Malformed,
        ErrorKind::Malformed,
    ]
    .into_iter()
    .enumerate()
    {
        if index > 0 {
            call.call = CallGeneration::from_local(index as u128 + 1);
            diagnostics::admit_fixture(
                &mut owner,
                &context(&runtime),
                call,
                "selected-session",
                "old-sid",
            );
            selected = owner
                .diagnostic_take_selected_peer(&context(&runtime), call)
                .unwrap();
        }
        let failed = owner
            .diagnostic_poll_selected(&context(&runtime), call, Some(&selected))
            .await;
        assert_eq!(failed.err().unwrap().kind, kind);
    }
    call.call = CallGeneration::from_local(7);
    diagnostics::admit_fixture(
        &mut owner,
        &context(&runtime),
        call,
        "selected-session",
        "old-sid",
    );
    selected = owner
        .diagnostic_take_selected_peer(&context(&runtime), call)
        .unwrap();
    let no_selection = owner
        .diagnostic_poll(&context(&runtime), call)
        .await
        .unwrap();
    assert!(no_selection.inspect().unwrap().membership_observed);
    assert_eq!(
        no_selection.inspect().unwrap().selected_peer_present,
        Some(false)
    );
    drop(no_selection);
    let count = server.count();
    let new_call = CallScope {
        call: CallGeneration::from_local(99),
        ..call
    };
    owner.diagnostic_membership_scope = Some((new_call, 7));
    assert_eq!(
        owner
            .diagnostic_poll_selected(&context(&runtime), call, Some(&selected))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(server.count(), count);
    owner.observed_call = Some(new_call);
    // A new observed call cannot inherit an older numeric-room eligibility.
    assert_eq!(
        owner
            .diagnostic_poll_selected(&context(&runtime), new_call, Some(&selected))
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
    assert_eq!(
        owner
            .diagnostic_poll(&context(&runtime), call)
            .await
            .err()
            .unwrap()
            .kind,
        ErrorKind::WrongScope
    );
    assert!(owner.diagnostic_membership_scope.is_none());
    assert_eq!(server.count(), count);
}

#[tokio::test]
async fn membership_binding_requires_successful_join_and_closes_before_failed_leave() {
    use talk_core::ids::CallGeneration;
    for number in [Value::Null, json!("7"), json!(0), json!(-1), json!(7)] {
        let mut joining = room();
        joining["id"] = number.clone();
        let mut replies = vec![
            Reply::data(caps(35)),
            Reply::data(room()),
            Reply::data(json!({"sessionId":"synthetic-session"})),
            Reply::data(caps(35)),
            Reply::data(joining),
        ];
        if number == json!(7) {
            replies.extend([
                Reply::data(json!({"signalingMode":"internal","stunservers":[],"turnservers":[]})),
                Reply::data(json!([])),
                Reply::envelope(500, "failure", 500, json!([])),
            ]);
        }
        let server = Server::start(replies).await;
        let runtime = runtime();
        let mut owner = client(&server.origin, runtime.clone());
        let conversation = bind(&mut owner);
        let call = CallScope {
            account: scope(),
            conversation,
            call: CallGeneration::from_local(1),
        };
        owner.diagnostic_membership_scope = Some((
            CallScope {
                call: CallGeneration::from_local(99),
                ..call
            },
            99,
        ));
        let joined = owner
            .diagnostic_join_synthetic_call(&context(&runtime), call)
            .await;
        if number == json!(7) {
            joined.unwrap();
            assert_eq!(owner.diagnostic_membership_scope, Some((call, 7)));
            assert!(
                owner
                    .execute(&context(&runtime), ProtocolRequest::Leave(call))
                    .await
                    .is_err()
            );
        } else {
            assert_eq!(joined.err().unwrap().kind, ErrorKind::Malformed);
        }
        assert!(owner.diagnostic_membership_scope.is_none());
        // Cancellation also closes local membership eligibility before any new I/O.
        owner.diagnostic_membership_scope = Some((call, 7));
        owner.cancellation().cancel();
        let count = server.count();
        assert_eq!(
            owner
                .diagnostic_poll(&context(&runtime), call)
                .await
                .err()
                .unwrap()
                .kind,
            ErrorKind::Cancelled
        );
        assert!(owner.diagnostic_membership_scope.is_none());
        assert_eq!(server.count(), count);
    }
}

#[tokio::test]
async fn diagnostic_response_slots_survive_clients_and_are_global_to_runtime() {
    let server = Server::start(vec![]).await;
    let runtime = runtime();
    let owner = client(&server.origin, runtime.clone());
    let mut held = Vec::new();
    for _ in 0..4 {
        held.push(owner.transport.diagnostic_response_slot().unwrap());
    }
    drop(owner);
    let replacement = client(&server.origin, runtime.clone());
    for _ in 0..4 {
        held.push(replacement.transport.diagnostic_response_slot().unwrap());
    }
    assert_eq!(
        replacement
            .transport
            .diagnostic_response_slot()
            .unwrap_err()
            .kind,
        ErrorKind::Busy
    );
    drop(held.pop());
    let available = replacement.transport.diagnostic_response_slot().unwrap();
    assert_eq!(
        replacement
            .transport
            .diagnostic_response_slot()
            .unwrap_err()
            .kind,
        ErrorKind::Busy
    );
    assert_eq!(server.count(), 0);
    drop((available, held));
    assert!(replacement.transport.diagnostic_response_slot().is_ok());
}

#[tokio::test]
async fn diagnostic_admission_tombstones_survive_failure_leave_cancel_and_replacement() {
    use talk_core::ids::CallGeneration;
    let server = Server::start(vec![Reply::envelope(500, "failure", 500, json!([]))]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let call = CallScope {
        account: scope(),
        conversation: bind(&mut owner),
        call: CallGeneration::from_local(u128::MAX),
    };
    let admitted = owner
        .transport
        .diagnostic_call_admission(&context(&runtime), call)
        .unwrap();
    assert_eq!(admitted.scope(), call);
    // A failed remote operation cannot refund admission. The full diagnostic
    // join/state consumer has separate media-owned tests on its combined input.
    assert!(
        owner
            .execute(&context(&runtime), ProtocolRequest::ProbeAccount)
            .await
            .is_err()
    );
    let _ = admitted;
    let second = CallScope {
        call: CallGeneration::from_local(0),
        ..call
    };
    let admitted_second = owner
        .transport
        .diagnostic_call_admission(&context(&runtime), second)
        .unwrap();
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(&context(&runtime), call)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Conflict
    );
    // Leave without a live call does no I/O, and cannot erase either tombstone.
    let _ = owner
        .execute(&context(&runtime), ProtocolRequest::Leave(second))
        .await;
    owner.cancellation().cancel();
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(&context(&runtime), call)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
    drop((admitted_second, owner));
    let replacement = client(&server.origin, runtime.clone());
    for old in [call, second, call] {
        assert_eq!(
            replacement
                .transport
                .diagnostic_call_admission(&context(&runtime), old)
                .err()
                .unwrap()
                .kind,
            ErrorKind::Conflict
        );
    }
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn diagnostic_admission_has_eight_total_slots_across_accounts_and_no_eviction() {
    use talk_core::ids::CallGeneration;
    let server = Server::start(vec![]).await;
    let runtime = runtime();
    let mut first = None;
    for index in 0..8 {
        let account = AccountScope {
            account: AccountId::from_local(index + 1),
            runtime: RuntimeGeneration::from_local(u128::MAX - index),
        };
        let mut owner = NextcloudClient::from_identity(
            AuthenticatedBinding {
                scope: account,
                host: HostId::from_local(1),
                origin: transport::ServerOrigin::loopback(&server.origin).unwrap(),
                login_name: Zeroizing::new("fixture_user".into()),
                user: Zeroizing::new("fixture_user".into()),
                password: Zeroizing::new("canary-password-not-for-logs".into()),
                cancellation: Cancellation::default(),
            },
            runtime.clone(),
        )
        .unwrap();
        let call = CallScope {
            account,
            conversation: bind(&mut owner),
            call: CallGeneration::from_local(index),
        };
        let mut request = context(&runtime);
        request.scope = account;
        let _admitted = owner
            .transport
            .diagnostic_call_admission(&request, call)
            .unwrap();
        if index == 0 {
            first = Some((call, owner));
        }
    }
    let (call, owner) = first.unwrap();
    let mut request = context(&runtime);
    request.scope = call.account;
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(&request, call)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(
                &request,
                CallScope {
                    call: CallGeneration::from_local(99),
                    ..call
                }
            )
            .err()
            .unwrap()
            .kind,
        ErrorKind::Busy
    );
    // Response reservations are independently available after lifetime exhaustion.
    assert!(owner.transport.diagnostic_response_slot().is_ok());
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn diagnostic_admission_rejects_invalid_context_without_burning_or_io() {
    use talk_core::ids::CallGeneration;
    let server = Server::start(vec![]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let call = CallScope {
        account: scope(),
        conversation: bind(&mut owner),
        call: CallGeneration::from_local(1),
    };
    for wrong in [
        CallScope {
            account: AccountScope {
                runtime: RuntimeGeneration::from_local(2),
                ..scope()
            },
            ..call
        },
        CallScope {
            conversation: ConversationId {
                account: AccountId::from_local(2),
                ..call.conversation
            },
            ..call
        },
        CallScope {
            conversation: ConversationId {
                host: HostId::from_local(2),
                ..call.conversation
            },
            ..call
        },
    ] {
        assert_eq!(
            owner
                .transport
                .diagnostic_call_admission(&context(&runtime), wrong)
                .err()
                .unwrap()
                .kind,
            ErrorKind::WrongScope
        );
    }
    let mut expired = context(&runtime);
    expired.deadline.epoch = ClockEpoch::from_local(2);
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(&expired, call)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Deadline
    );
    let cancelled = context(&runtime);
    cancelled.cancellation.cancel();
    assert_eq!(
        owner
            .transport
            .diagnostic_call_admission(&cancelled, call)
            .err()
            .unwrap()
            .kind,
        ErrorKind::Cancelled
    );
    assert!(
        owner
            .transport
            .diagnostic_call_admission(&context(&runtime), call)
            .is_ok()
    );
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn concurrent_duplicate_diagnostic_admission_is_atomic() {
    use talk_core::ids::CallGeneration;
    let server = Server::start(vec![]).await;
    let runtime = runtime();
    let mut owner = client(&server.origin, runtime.clone());
    let call = CallScope {
        account: scope(),
        conversation: bind(&mut owner),
        call: CallGeneration::from_local(1),
    };
    let request = context(&runtime);
    let barrier = std::sync::Barrier::new(8);
    let results = std::thread::scope(|threads| {
        let workers: Vec<_> = (0..8)
            .map(|_| {
                threads.spawn(|| {
                    barrier.wait();
                    owner
                        .transport
                        .diagnostic_call_admission(&request, call)
                        .map(|_| ())
                        .map_err(|error| error.kind)
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter(|result| **result == Err(ErrorKind::Conflict))
            .count(),
        7
    );
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn account_rejection_requires_complete_consistent_pinned_v2_evidence() {
    let valid =
        json!({"ocs":{"meta":{"status":"failure","statuscode":997,"message":""},"data":[]}});
    let mut cases = Vec::new();
    for (key, values) in [
        (
            "status",
            vec![Value::Null, json!(3), json!("unknown"), json!("ok")],
        ),
        (
            "statuscode",
            vec![
                Value::Null,
                json!("997"),
                json!(997.0),
                json!(-1),
                json!(401),
                json!(403),
                json!(200),
                json!(12345),
            ],
        ),
        ("message", vec![Value::Null, json!(true)]),
    ] {
        for value in values {
            let mut body = valid.clone();
            body["ocs"]["meta"][key] = value;
            cases.push((401, body));
        }
        let mut body = valid.clone();
        body["ocs"]["meta"].as_object_mut().unwrap().remove(key);
        cases.push((401, body));
    }
    let mut contradictory = valid.clone();
    contradictory["ocs"]["meta"]["status"] = json!("ok");
    contradictory["ocs"]["meta"]["statuscode"] = json!(200);
    cases.push((401, contradictory));
    for data in [Value::Null, json!("unknown"), json!(12)] {
        let mut body = valid.clone();
        body["ocs"]["data"] = data;
        cases.push((401, body));
    }
    let mut absent = valid.clone();
    absent["ocs"].as_object_mut().unwrap().remove("data");
    cases.push((401, absent));
    cases.push((200, valid.clone()));
    for (status, body) in cases {
        let mut reply = Reply::data(json!({}));
        reply.status = status;
        reply.body = serde_json::to_vec(&body).unwrap();
        let server = Server::start(vec![reply]).await;
        let rt = runtime();
        let mut owner = client(&server.origin, rt.clone());
        let cancellation = owner.cancellation();
        assert!(
            matches!(owner.request(context(&rt),ProtocolRequest::ProbeAccount).await,Err(e)if e.kind==ErrorKind::Malformed)
        );
        assert!(!cancellation.is_cancelled());
        assert_eq!(owner.availability(), Availability::Ready);
    }
    let mut truncated = Reply::envelope(401, "failure", 997, json!([]));
    truncated.body.pop();
    let server = Server::start(vec![truncated]).await;
    let rt = runtime();
    let mut owner = client(&server.origin, rt.clone());
    let cancellation = owner.cancellation();
    assert!(
        matches!(owner.request(context(&rt),ProtocolRequest::ProbeAccount).await,Err(e)if e.kind==ErrorKind::Malformed)
    );
    assert!(!cancellation.is_cancelled());
}

#[tokio::test]
async fn authoritative_rejection_is_never_inferred_on_an_ordinary_route() {
    let server = Server::start(vec![
        Reply::envelope(401, "failure", 997, json!([])),
        Reply::envelope(401, "failure", 997, json!([])),
    ])
    .await;
    let rt = runtime();
    let mut owner = client(&server.origin, rt.clone());
    let cancellation = owner.cancellation();
    assert!(
        matches!(owner.transport.ocs(&context(&rt),Method::GET,"ocs/v2.php/apps/spreed/api/v4/room",&[],&[],true).await,Err(e)if e.kind==ErrorKind::WrongScope)
    );
    assert_eq!(server.count(), 0);
    assert!(
        matches!(owner.transport.ocs(&context(&rt),Method::GET,"ocs/v2.php/apps/spreed/api/v4/room",&[],&[],false).await,Err(e)if e.kind==ErrorKind::PermissionDenied)
    );
    assert!(!cancellation.is_cancelled());
    assert!(
        matches!(owner.request(context(&rt),ProtocolRequest::ProbeAccount).await,Err(e)if e.kind==ErrorKind::ReauthenticationRequired)
    );
    assert!(cancellation.is_cancelled());
    assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn common_read_requires_explicit_public_privacy() {
    for privacy in [Value::Null, json!(1), json!(0)] {
        let mut capabilities = caps(34);
        capabilities["capabilities"]["spreed"]["config"]["chat"]["read-privacy"] = privacy.clone();
        let mut reply = Reply::data(json!([message()]));
        reply.headers = "X-Chat-Last-Common-Read: 4\r\n".into();
        let server =
            Server::start(vec![Reply::data(capabilities), Reply::data(room()), reply]).await;
        let runtime = runtime();
        let mut client = client(&server.origin, runtime.clone());
        let conversation = bind(&mut client);
        let ProtocolReply::History(history) = client
            .request(
                context(&runtime),
                ProtocolRequest::History {
                    conversation,
                    before: None,
                },
            )
            .await
            .unwrap()
        else {
            panic!("history");
        };
        assert_eq!(history.common_read.is_some(), privacy == json!(0));
    }
}
