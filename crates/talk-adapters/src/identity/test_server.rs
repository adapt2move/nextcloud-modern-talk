//! Isolated loopback protocol fixture. Only random synthetic secrets, never printed.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use zeroize::Zeroizing;
pub(super) struct Server {
    pub origin: String,
    pub authorized: Arc<AtomicBool>,
    pub revoked: Arc<AtomicBool>,
    pub probe_override: Arc<Mutex<Option<(u16, serde_json::Value)>>>,
    pub wrong_user: Arc<AtomicBool>,
    pub polls: Arc<AtomicUsize>,
    pub authenticated: Arc<AtomicUsize>,
    pub delayed: Arc<AtomicBool>,
    pub rate_limited: Arc<AtomicBool>,
    pub user_probes: Arc<AtomicUsize>,
    pub validation_pause: Arc<AtomicBool>,
    pub validation_entered: Arc<AtomicBool>,
    pub validation_release: Arc<tokio::sync::Notify>,
    pub mutations: Arc<AtomicUsize>,
    job: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.job.abort();
    }
}
fn base64(bytes: &[u8]) -> Zeroizing<String> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = Zeroizing::new(String::new());
    for b in bytes.chunks(3) {
        let n = (u32::from(b[0]) << 16)
            | (u32::from(*b.get(1).unwrap_or(&0)) << 8)
            | u32::from(*b.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i > b.len() {
                text.push('=');
            } else {
                text.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
    }
    text
}
impl Server {
    pub async fn start(major: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/cloud/", listener.local_addr().unwrap());
        let authorized = Arc::new(AtomicBool::new(false));
        let revoked = Arc::new(AtomicBool::new(false));
        let probe_override = Arc::new(Mutex::new(None::<(u16, serde_json::Value)>));
        let override_probe = probe_override.clone();
        let wrong_user = Arc::new(AtomicBool::new(false));
        let polls = Arc::new(AtomicUsize::new(0));
        let authenticated = Arc::new(AtomicUsize::new(0));
        let delayed = Arc::new(AtomicBool::new(false));
        let rate_limited = Arc::new(AtomicBool::new(false));
        let user_probes = Arc::new(AtomicUsize::new(0));
        let validation_pause = Arc::new(AtomicBool::new(false));
        let validation_entered = Arc::new(AtomicBool::new(false));
        let validation_release = Arc::new(tokio::sync::Notify::new());
        let (pause, entered, release) = (
            validation_pause.clone(),
            validation_entered.clone(),
            validation_release.clone(),
        );
        let mutations = Arc::new(AtomicUsize::new(0));
        let (rate, probes, writes) = (rate_limited.clone(), user_probes.clone(), mutations.clone());
        let (url, ok, invalid, wrong, count, auth, delay) = (
            origin.clone(),
            authorized.clone(),
            revoked.clone(),
            wrong_user.clone(),
            polls.clone(),
            authenticated.clone(),
            delayed.clone(),
        );
        let job = tokio::spawn(async move {
            let token = Zeroizing::new(format!("{:032x}", super::ledger::random().unwrap()));
            let password = Zeroizing::new(format!("{:032x}", super::ledger::random().unwrap()));
            let token = token.as_str();
            let password = password.as_str();
            let expected = Zeroizing::new(format!(
                "authorization: Basic {}",
                base64(
                    Zeroizing::new(format!("Synthetic.Login@Example.invalid:{password}"))
                        .as_bytes()
                )
                .as_str()
            ));
            for _ in 0..256 {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut input = Zeroizing::new(Vec::new());
                loop {
                    let mut bytes = Zeroizing::new([0; 1024]);
                    let n = stream.read(&mut *bytes).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    input.extend_from_slice(&bytes[..n]);
                    assert!(input.len() <= 16384, "fixture request bound");
                    if let Some(end) = input.windows(4).position(|b| b == b"\r\n\r\n") {
                        let header =
                            Zeroizing::new(String::from_utf8_lossy(&input[..end]).into_owned());
                        let len = header
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length: "))
                            .map(|n| n.parse::<usize>().unwrap())
                            .unwrap_or(0);
                        if input.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                let input = Zeroizing::new(String::from_utf8(input.to_vec()).unwrap());
                let path = input
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let (code, body) = if path == "/cloud/index.php/login/v2" {
                    (
                        200,
                        Zeroizing::new(format!(
                            r#"{{"poll":{{"token":"{token}","endpoint":"{url}index.php/login/v2/poll"}},"login":"{url}index.php/login/v2/flow/{token}"}}"#
                        )),
                    )
                } else if path == "/cloud/index.php/login/v2/poll" {
                    count.fetch_add(1, Ordering::AcqRel);
                    assert!(
                        input.ends_with(&format!("token={token}")),
                        "fixture token binding"
                    );
                    if delay.load(Ordering::Acquire) {
                        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                    }
                    if ok.load(Ordering::Acquire) {
                        (
                            200,
                            Zeroizing::new(format!(
                                r#"{{"server":"{url}","loginName":"Synthetic.Login@Example.invalid","appPassword":"{password}"}}"#
                            )),
                        )
                    } else {
                        (404, Zeroizing::new("{}".into()))
                    }
                } else {
                    assert!(
                        input.lines().any(|l| l == expected.as_str()),
                        "app credential must originate in the grant"
                    );
                    auth.fetch_add(1, Ordering::AcqRel);
                    if path == "/cloud/ocs/v2.php/cloud/user?format=json" {
                        probes.fetch_add(1, Ordering::AcqRel);
                        if pause.load(Ordering::Acquire) {
                            entered.store(true, Ordering::Release);
                            while pause.load(Ordering::Acquire) {
                                release.notified().await;
                            }
                        }
                        if let Some((code, body)) = override_probe.lock().unwrap().as_ref() {
                            (*code, Zeroizing::new(body.to_string()))
                        } else if rate.load(Ordering::Acquire) {
                            (429,Zeroizing::new(r#"{"ocs":{"meta":{"status":"failure","statuscode":429},"data":{}}}"#.into()))
                        } else if invalid.load(Ordering::Acquire) {
                            (401,Zeroizing::new(r#"{"ocs":{"meta":{"status":"failure","statuscode":997,"message":""},"data":{}}}"#.into()))
                        } else {
                            let user = if wrong.load(Ordering::Acquire) {
                                "different-user"
                            } else {
                                "synthetic-user"
                            };
                            (
                                200,
                                Zeroizing::new(format!(
                                    r#"{{"ocs":{{"meta":{{"status":"ok","statuscode":200}},"data":{{"id":"{user}","displayname":"Synthetic"}}}}}}"#
                                )),
                            )
                        }
                    } else if path == "/cloud/ocs/v2.php/cloud/capabilities?format=json" {
                        (200,Zeroizing::new(serde_json::json!({"ocs":{"meta":{"status":"ok","statuscode":200},"data":{"version":{"major":major},"capabilities":{"spreed":{"features":["conversation-v4","chat-reference-id"],"config":{"conversations":{"can-create":true},"chat":{"max-length":32000}}}}}}}).to_string()))
                    } else {
                        if input.starts_with("POST ") {
                            writes.fetch_add(1, Ordering::AcqRel);
                        }
                        (503,Zeroizing::new(r#"{"ocs":{"meta":{"status":"failure","statuscode":503},"data":{}}}"#.into()))
                    }
                };
                assert!(
                    serde_json::from_str::<serde_json::Value>(&body).is_ok(),
                    "synthetic response JSON invalid"
                );
                let header = format!(
                    "HTTP/1.1 {code} Fixture\r\nContent-Type: application/json\r\nRetry-After: 1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                if stream.write_all(header.as_bytes()).await.is_ok() {
                    let _ = stream.write_all(body.as_bytes()).await;
                }
            }
        });
        Self {
            origin,
            authorized,
            revoked,
            probe_override,
            wrong_user,
            polls,
            authenticated,
            delayed,
            rate_limited,
            user_probes,
            validation_pause,
            validation_entered,
            validation_release,
            mutations,
            job,
        }
    }
}
