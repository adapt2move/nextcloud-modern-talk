//! Actual native process checks for the private typed media owner seam.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/runtime.rs"]
#[allow(dead_code)]
mod runtime;
#[path = "../../crates/talk-adapters/src/media/supervisor.rs"]
#[allow(dead_code)]
mod supervisor;
#[path = "../../crates/talk-adapters/src/media/wire.rs"]
#[allow(dead_code)]
mod wire;
use runtime::{Error, Intent, Observation, SyntheticConfig, SyntheticIntent, SyntheticMedia};
use std::path::Path;
use std::time::{Duration, Instant};
use talk_core::ids::{
    AccountId, AccountScope, CallGeneration, CallScope, ConversationId, HostId, RuntimeGeneration,
};
use talk_core::lifecycle::Cancellation;
fn call(value: u8) -> (CallScope, wire::Scope) {
    let id = u128::from_be_bytes([value; 16]);
    (
        CallScope {
            account: AccountScope {
                account: AccountId::from_local(id),
                runtime: RuntimeGeneration::from_local(id),
            },
            conversation: ConversationId {
                account: AccountId::from_local(id),
                host: HostId::from_local(id),
                local: 1,
            },
            call: CallGeneration::from_local(id),
        },
        wire::Scope([value; 64]),
    )
}
fn next(media: &mut SyntheticMedia, call: CallScope) -> Observation {
    let start = Instant::now();
    loop {
        assert!(start.elapsed() < Duration::from_secs(2));
        if let Some(event) = media.advance(call).unwrap() {
            return event;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn main() {
    let args: Vec<_> = std::env::args_os().take(3).collect();
    assert_eq!(args.len(), 2);
    let helper = Path::new(&args[1]);
    let (scope, wire) = call(31);
    let config = SyntheticConfig::new(
        scope,
        wire,
        Cancellation::default(),
        SyntheticIntent::ObserveOnly,
    )
    .unwrap();
    let mut media = SyntheticMedia::start(helper, config).unwrap();
    assert!(matches!(next(&mut media, scope), Observation::Ready));
    let mut other = scope;
    other.conversation.local += 1;
    assert!(matches!(
        media.submit(other, Intent::Offer),
        Err(runtime::Rejected {
            reason: Error::WrongScope,
            ..
        })
    ));
    assert!(media.submit(scope, Intent::Inspect).is_ok());
    let Observation::Counters(counts) = next(&mut media, scope) else {
        panic!("counters absent")
    };
    assert_eq!(counts[0], 0);
    assert_eq!(counts[2], 0);
    // A large description owns one staging slot without emitting partial chunks
    // from submit. Backpressure returns the intact next intent; stop bypasses it.
    let text = format!("v=0\r\n{}", "a=x\r\n".repeat(12000));
    assert!(
        media
            .submit(
                scope,
                Intent::RemoteDescription(
                    wire::SessionDescription::new(wire::DescriptionKind::Offer, &text).unwrap()
                )
            )
            .is_ok()
    );
    assert!(matches!(
        media.submit(scope, Intent::Inspect),
        Err(runtime::Rejected {
            reason: Error::Busy,
            intent: Intent::Inspect
        })
    ));
    let control = media.control();
    assert_eq!(control.close(other), Err(Error::WrongScope));
    let start = Instant::now();
    control.close(scope).unwrap();
    assert!(matches!(media.advance(scope), Err(Error::Stopped)));
    while media.outcome() == supervisor::Exit::Running {
        assert!(start.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(media.outcome(), supervisor::Exit::DiagnosticDrained);
    println!(
        "typed owner: scope rejection, single-description backpressure, priority stop pass; elapsed_ms={}",
        start.elapsed().as_millis()
    );
    let (scope, wire) = call(32);
    let cancelled = Cancellation::default();
    let mut media = SyntheticMedia::start(
        helper,
        SyntheticConfig::new(scope, wire, cancelled.clone(), SyntheticIntent::ObserveOnly).unwrap(),
    )
    .unwrap();
    assert!(matches!(next(&mut media, scope), Observation::Ready));
    cancelled.cancel();
    assert!(matches!(media.advance(scope), Err(Error::Cancelled)));
    // Existing account owner must also invoke Control.close independently of
    // pump progress; shared cancellation here qualifies only the admission check.
    let start = Instant::now();
    while media.outcome() == supervisor::Exit::Running {
        assert!(start.elapsed() < Duration::from_secs(1));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(media.outcome(), supervisor::Exit::DiagnosticDrained);
    println!(
        "typed owner: shared account cancellation rejects observations and drains synthetic native child"
    );
}
