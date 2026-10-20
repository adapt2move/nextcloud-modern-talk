//! Standalone diagnostic composition of the actual safe Rust supervisor.
//! Compile with the pinned rustc; no extra facade crate or Cargo graph mutation.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/supervisor.rs"]
mod supervisor;
#[path = "../../crates/talk-adapters/src/media/wire.rs"]
#[allow(dead_code)]
mod wire;
use std::io::{self, Write};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};
use supervisor::{DiagnosticSession, Exit};
use wire::{Kind, Record};

fn main() {
    let args: Vec<_> = std::env::args_os().take(4).collect();
    if args.len() != 3 || args[1] != "--diagnostic-synthetic" {
        std::process::exit(64);
    }
    let mut input = io::stdin();
    let create = match Record::read(&mut input) {
        Ok(value) => value,
        Err(_) => std::process::exit(64),
    };
    let mut session = match DiagnosticSession::start(Path::new(&args[2]), create) {
        Ok(value) => value,
        Err(_) => std::process::exit(65),
    };
    let (commands, queue) = mpsc::sync_channel(16);
    let stop = Arc::new(AtomicBool::new(false));
    let cause = Arc::new(AtomicU8::new(0));
    let reader_cause = cause.clone();
    let reader_stop = stop.clone();
    std::thread::spawn(move || {
        for _ in 0..512 {
            let record = match Record::read(&mut input) {
                Ok(value) => value,
                Err(_) => {
                    let _ =
                        reader_cause.compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire);
                    break;
                }
            };
            if record.kind == Kind::Stop {
                let _ = reader_cause.compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire);
                reader_stop.store(true, Ordering::Release);
                return;
            }
            if commands.try_send(record).is_err() {
                let _ = reader_cause.compare_exchange(0, 3, Ordering::AcqRel, Ordering::Acquire);
                break;
            }
        }
        reader_stop.store(true, Ordering::Release);
    });
    let started = Instant::now();
    let mut output = io::stdout();
    loop {
        if stop.load(Ordering::Acquire) || started.elapsed() > Duration::from_secs(30) {
            if !stop.load(Ordering::Acquire) {
                let _ = cause.compare_exchange(0, 4, Ordering::AcqRel, Ordering::Acquire);
            }
            session.revoke();
        }
        if let Ok(command) = queue.try_recv()
            && session.submit(command).is_err()
        {
            let _ = cause.compare_exchange(0, 5, Ordering::AcqRel, Ordering::Acquire);
            session.revoke();
        }
        if let Some(event) = session.receive(Duration::from_millis(5))
            && event
                .write(&mut output)
                .and_then(|_| output.flush())
                .is_err()
        {
            let _ = cause.compare_exchange(0, 6, Ordering::AcqRel, Ordering::Acquire);
            session.revoke();
        }
        if session.outcome() != Exit::Running {
            break;
        }
    }
    while let Some(event) = session.receive(Duration::ZERO) {
        let _ = event.write(&mut output).and_then(|_| output.flush());
    }
    // No SDP, candidate, private path, device data or raw native error.
    eprintln!("native diagnostic outcome: {:?}", session.outcome());
    eprintln!(
        "native diagnostic stop cause: {}",
        match cause.load(Ordering::Acquire) {
            0 => "worker-or-native",
            1 => "input-ended-or-malformed",
            2 => "explicit-stop",
            3 => "input-queue-unavailable",
            4 => "absolute-expiry",
            5 => "command-rejected",
            6 => "output-unavailable",
            _ => "unknown",
        }
    );
}
