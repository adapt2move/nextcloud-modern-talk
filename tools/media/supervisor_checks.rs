//! Executable actual-helper lifecycle checks; uses no production authority.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/supervisor.rs"]
#[allow(dead_code)]
mod supervisor;
#[path = "../../crates/talk-adapters/src/media/wire.rs"]
#[allow(dead_code)]
mod wire;
use std::path::Path;
use std::time::{Duration, Instant};
use supervisor::{DiagnosticSession, Error, Exit};
use wire::{Kind, Record, Scope};

fn create(helper: &Path, scope: Scope) -> Result<DiagnosticSession, Error> {
    DiagnosticSession::start(
        helper,
        Record::new(Kind::Create, scope, 1, 1, &[0]).unwrap(),
    )
}
fn ready(session: &DiagnosticSession) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        assert!(Instant::now() < deadline, "native ready timed out");
        if let Some(event) = session.receive(Duration::from_millis(10)) {
            assert_eq!(event.kind, Kind::Ready);
            return;
        }
    }
}
fn drain(session: &DiagnosticSession) {
    let deadline = Instant::now() + Duration::from_secs(1);
    session.revoke();
    while session.outcome() == Exit::Running {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(session.outcome(), Exit::DiagnosticDrained);
}
fn main() {
    let args: Vec<_> = std::env::args_os().take(4).collect();
    assert!((2..=3).contains(&args.len()));
    let helper = Path::new(&args[1]);
    if args.get(2).is_some_and(|arg| arg == "--absolute-expiry") {
        let started = Instant::now();
        let scope = Scope([8; 64]);
        let mut session = create(helper, scope).unwrap();
        ready(&session);
        let mut serial = 1;
        let mut next_stats = Instant::now();
        let mut revoked_at = None;
        while session.outcome() == Exit::Running {
            assert!(started.elapsed() < Duration::from_millis(30_500));
            if Instant::now() >= next_stats {
                serial += 1;
                let _ =
                    session.submit(Record::new(Kind::Stats, scope, serial, serial, &[]).unwrap());
                next_stats = Instant::now() + Duration::from_millis(200);
            }
            if let Some(event) = session.receive(Duration::from_millis(5))
                && event.kind == Kind::Counters
            {
                let value = u64::from_be_bytes(event.payload()[72..80].try_into().unwrap());
                if value != 0 {
                    revoked_at = Some(value);
                }
            }
        }
        let elapsed = started.elapsed();
        assert!(elapsed >= Duration::from_millis(29_800));
        assert_eq!(session.outcome(), Exit::DiagnosticDrained);
        assert_eq!(
            session.submit(Record::new(Kind::Stats, scope, serial + 1, serial + 1, &[]).unwrap()),
            Err(Error::Stale)
        );
        assert!(matches!(create(helper, scope), Err(Error::Stale)));
        println!(
            "absolute synthetic expiry: pass; elapsed_ms={}; native_revoke_us={revoked_at:?}; late command and same-scope recreation rejected",
            elapsed.as_millis()
        );
        return;
    }
    let a = Scope([1; 64]);
    let b = Scope([2; 64]);
    let c = Scope([3; 64]);
    let mut first = create(helper, a).unwrap();
    ready(&first);
    let second = create(helper, b).unwrap();
    ready(&second);
    assert!(matches!(create(helper, c), Err(Error::Busy)));
    let mut same_account = a;
    same_account.0[48..].fill(9);
    assert!(matches!(create(helper, same_account), Err(Error::Busy)));
    assert_eq!(
        first.submit(Record::new(Kind::Stats, b, 2, 2, &[]).unwrap()),
        Err(Error::WrongScope)
    );
    first
        .submit(Record::new(Kind::Stats, a, 2, 2, &[]).unwrap())
        .unwrap();
    assert_eq!(
        first.submit(Record::new(Kind::Stats, a, 2, 2, &[]).unwrap()),
        Err(Error::Stale)
    );
    let counters = first
        .receive(Duration::from_millis(50))
        .expect("native counters");
    assert_eq!(counters.kind, Kind::Counters);
    for offset in [0, 16] {
        assert_eq!(
            u64::from_be_bytes(counters.payload()[offset..offset + 8].try_into().unwrap()),
            0,
            "no source intent must produce no synthetic capture"
        );
    }
    // Revoke returns before teardown. Replacement remains charged until the
    // real child and both IPC workers settle, not merely until a flag is set.
    first.revoke();
    assert!(matches!(create(helper, same_account), Err(Error::Busy)));
    drain(&first);
    assert!(matches!(create(helper, a), Err(Error::Stale)));
    let replacement = create(helper, same_account).unwrap();
    ready(&replacement);
    drain(&replacement);
    drain(&second);
    println!(
        "actual native supervisor checks: pass (two slots; same-account/retiring rejection; wrong scope; stale serial; no intent; drain and replacement)"
    );
}
