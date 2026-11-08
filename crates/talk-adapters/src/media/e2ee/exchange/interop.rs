//! Drives this implementation against Talk's own JavaScript key-exchange logic
//! (`tools/e2ee/olm_peer.mjs`, running libolm) to show both sides agree.
//!
//! The test needs `node` and `npm ci` in `tools/e2ee`; without them it reports why
//! it is skipped and passes, so ordinary checkouts are unaffected.
use super::*;
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn tool_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/e2ee")
}

struct Peer {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    keys: HashMap<String, (Vec<u8>, u32)>,
    local: Option<(Vec<u8>, u32)>,
}
impl Peer {
    fn start(session: &str) -> Option<Self> {
        if !tool_dir().join("node_modules/@matrix-org/olm").exists() {
            eprintln!("skipped: run `npm ci` in tools/e2ee to enable the libolm interop test");
            return None;
        }
        let mut child = Command::new("node")
            .arg("olm_peer.mjs")
            .current_dir(tool_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|error| eprintln!("skipped: node is not available ({error})"))
            .ok()?;
        let input = child.stdin.take()?;
        let output = BufReader::new(child.stdout.take()?);
        let mut peer = Self {
            child,
            input,
            output,
            keys: HashMap::new(),
            local: None,
        };
        peer.command(&json!({"cmd": "init", "session": session}));
        Some(peer)
    }
    /// Send a command and collect everything the peer printed until it is idle.
    fn command(&mut self, command: &Value) -> Vec<Value> {
        writeln!(self.input, "{command}").unwrap();
        // A marker command makes the peer's reply stream end deterministically.
        writeln!(self.input, "{}", json!({"cmd": "ping"})).unwrap();
        let mut sends = Vec::new();
        loop {
            let mut line = String::new();
            self.output.read_line(&mut line).unwrap();
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                break;
            };
            if value.get("pong").is_some() {
                break;
            }
            if let Some(key) = value.get("key") {
                self.keys.insert(
                    key["peer"].as_str().unwrap().to_owned(),
                    (
                        base64_decode(key["key"].as_str().unwrap().trim_end_matches('=')).unwrap(),
                        key["index"].as_u64().unwrap() as u32,
                    ),
                );
            }
            if let Some(local) = value.get("local") {
                self.local = Some((
                    base64_decode(local["key"].as_str().unwrap().trim_end_matches('=')).unwrap(),
                    local["index"].as_u64().unwrap() as u32,
                ));
            }
            assert!(value.get("error").is_none(), "peer error: {value}");
            if let Some(send) = value.get("send") {
                sends.push(send.clone());
            }
        }
        sends
    }
}
impl Drop for Peer {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "{}", json!({"cmd": "quit"}));
        let _ = self.child.wait();
    }
}

type Seen = HashMap<String, ([u8; KEY_LEN], u32)>;

/// Take the Rust side's outputs: remember key updates, return what to send.
fn flush(
    rust: &mut Exchange,
    seen: &mut Seen,
    local: &mut Option<([u8; KEY_LEN], u32)>,
) -> Vec<(String, Value)> {
    let mut sends = Vec::new();
    for output in rust.take_outputs() {
        match output {
            Output::Send { to, payload } => sends.push((to, payload)),
            Output::SetKey {
                participant: Participant::Remote(peer),
                key,
                index,
            } => {
                seen.insert(peer, (key, index));
            }
            Output::SetKey {
                participant: Participant::Local,
                key,
                index,
            } => *local = Some((key, index)),
            Output::Forget(_) => {}
        }
    }
    sends
}

/// Deliver messages in both directions until neither side has more to say.
fn converse(
    now: Instant,
    rust: &mut Exchange,
    rust_id: &str,
    js: &mut Peer,
    js_id: &str,
    seen: &mut Seen,
    local: &mut Option<([u8; KEY_LEN], u32)>,
) {
    for _ in 0..50 {
        let sends = flush(rust, seen, local);
        if sends.is_empty() {
            return;
        }
        for (to, payload) in sends {
            assert_eq!(to, js_id);
            let replies =
                js.command(&json!({"cmd": "message", "from": rust_id, "payload": payload}));
            for reply in replies {
                rust.message(now, js_id, &reply["payload"]);
            }
        }
    }
    panic!("no convergence");
}

fn run(rust_id: &str, js_id: &str) {
    let Some(mut js) = Peer::start(js_id) else {
        return;
    };
    let now = Instant::now();
    let mut rust = Exchange::new(rust_id).unwrap();
    let (mut seen, mut local) = (Seen::new(), None);
    // The roster delivers both sides' view of the other.
    rust.users_joined(now, &[js_id.to_owned()]);
    let sends = js.command(&json!({"cmd": "joined", "sessions": [rust_id]}));
    for send in sends {
        rust.message(now, js_id, &send["payload"]);
    }
    converse(
        now, &mut rust, rust_id, &mut js, js_id, &mut seen, &mut local,
    );

    // Each side holds the key the other sends with.
    let js_local = js.local.clone().unwrap();
    let (key, index) = seen[js_id];
    assert_eq!((key.to_vec(), index), js_local, "JS key as seen by Rust");
    let (rust_key, rust_index) = js.keys[rust_id].clone();
    assert_eq!(rust_index, 0);
    assert_eq!(
        Some((rust_key, rust_index)),
        local.map(|(key, index)| (key.to_vec(), index))
    );

    // The JS side rotates its key; Rust receives and confirms it.
    let sends = js.command(&json!({"cmd": "rotate"}));
    for send in sends {
        rust.message(now, js_id, &send["payload"]);
    }
    converse(
        now, &mut rust, rust_id, &mut js, js_id, &mut seen, &mut local,
    );
    let js_local = js.local.clone().unwrap();
    assert_eq!(js_local.1, 1);
    assert_eq!(
        (seen[js_id].0.to_vec(), seen[js_id].1),
        js_local,
        "rotated JS key as seen by Rust"
    );

    // Rust rotates after a third participant leaves; JS receives the new key.
    rust.users_joined(now, &["zz-third".to_owned()]);
    flush(&mut rust, &mut seen, &mut local);
    rust.users_left(now, &["zz-third".to_owned()]);
    rust.poll(now + Duration::from_secs(6));
    converse(
        now, &mut rust, rust_id, &mut js, js_id, &mut seen, &mut local,
    );
    let (key, index) = local.expect("the rotation settled after JS confirmed");
    assert_eq!(index, 1);
    assert_eq!(
        js.keys[rust_id],
        (key.to_vec(), index),
        "rotated Rust key as seen by JS"
    );
}

#[test]
fn rust_starts_and_the_web_client_answers() {
    run("session-b-large", "session-a-small");
}

#[test]
fn the_web_client_starts_and_rust_answers() {
    run("session-a-small", "session-b-large");
}
