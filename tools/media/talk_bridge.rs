//! Explicit synthetic-only Talk/native interoperability diagnostic.
//! This example is exported only by backend-test. No production grant is made.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/supervisor.rs"]
#[allow(dead_code)]
mod supervisor;
#[path = "../../crates/talk-adapters/src/media/wire.rs"]
#[allow(dead_code)]
mod wire;

use serde_json::{Value, json};
use std::path::Path;
use std::time::{Duration, Instant};
use supervisor::Exit;
#[path = "../../crates/talk-adapters/src/media/runtime.rs"]
#[allow(dead_code)]
mod native_runtime;
use talk_adapters::nextcloud::{
    NetworkRuntime, NextcloudClient,
    diagnostics::{
        DiagnosticPeer, DiagnosticPoll, DiagnosticReadCompletion, DiagnosticWriteCompletion,
        Signal, SignalRef,
    },
    test_support,
};
use talk_core::bounded::Text;
use talk_core::ids::{
    AccountId, AccountScope, CallGeneration, CallScope, ClockEpoch, HostId, RuntimeGeneration,
};
use talk_core::lifecycle::Cancellation;
use talk_core::ports::{NextcloudPort, OperationContext, ProtocolRequest};
use wire::{DescriptionKind, IceCandidate, Scope, SessionDescription};

type Result<T> = std::result::Result<T, String>;
fn context(runtime: &NetworkRuntime, scope: AccountScope, millis: u64) -> OperationContext {
    let mut deadline = runtime.now();
    deadline.millis += millis;
    OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    }
}
fn bounded<const N: usize>(text: &str) -> Result<Text<N>> {
    Text::new(text).map_err(|_| "signal bounds".into())
}

struct Native {
    media: native_runtime::SyntheticMedia,
    call: CallScope,
    ready: bool,
    connected: bool,
    counters: [u64; 18],
    started: Instant,
    ready_ms: Option<u128>,
    sent_signals: u32,
    received_signals: u32,
    remote_profile: Value,
    offerer: Option<bool>,
    duplex: bool,
    leave: Value,
    deactivate: Value,
    polls: u8,
    writes_while_reading: u8,
    stopped_read: bool,
    stop_report: Option<Value>,
    expect_remote_leave: bool,
    remote_departure: bool,
    departure_observation: Value,
}
impl Native {
    fn start(helper: &Path, scope: Scope, call: CallScope) -> Result<Self> {
        let config = native_runtime::SyntheticConfig::new(
            call,
            scope,
            Cancellation::default(),
            native_runtime::SyntheticIntent::AudioVideo,
        )
        .map_err(|e| format!("native config {e:?}"))?;
        let media = native_runtime::SyntheticMedia::start(helper, config)
            .map_err(|e| format!("native start {e:?}"))?;
        Ok(Self {
            media,
            call,
            ready: false,
            connected: false,
            counters: [0; 18],
            started: Instant::now(),
            ready_ms: None,
            sent_signals: 0,
            received_signals: 0,
            remote_profile: Value::Null,
            offerer: None,
            duplex: false,
            leave: Value::Null,
            deactivate: Value::Null,
            polls: 0,
            writes_while_reading: 0,
            stopped_read: false,
            stop_report: None,
            expect_remote_leave: false,
            remote_departure: false,
            departure_observation: Value::Null,
        })
    }
    fn submit(&mut self, intent: native_runtime::Intent) -> Result<bool> {
        match self.media.submit(self.call, intent) {
            Ok(()) => Ok(true),
            Err(e) if e.reason == native_runtime::Error::Busy => Ok(false),
            Err(e) => Err(format!("native intent {:?}", e.reason)),
        }
    }
    fn apply(&mut self, signal: SignalRef<'_>) -> Result<bool> {
        let intent = match signal {
            SignalRef::Offer(text) | SignalRef::Answer(text) => {
                let kind = if matches!(signal, SignalRef::Offer(_)) {
                    DescriptionKind::Offer
                } else {
                    DescriptionKind::Answer
                };
                self.remote_profile = json!({"media_sections":text.lines().filter(|l|l.starts_with("m=")).count(),"application_section":text.lines().any(|l|l.starts_with("m=application ")),"simulcast":text.lines().any(|l|l.starts_with("a=simulcast")||l.starts_with("a=rid:"))});
                native_runtime::Intent::RemoteDescription(
                    SessionDescription::new(kind, text).map_err(|_| "remote SDP validation")?,
                )
            }
            SignalRef::Candidate {
                candidate,
                mid,
                index,
            } => native_runtime::Intent::RemoteCandidate(
                IceCandidate::new(mid, index, candidate).map_err(|_| "remote ICE validation")?,
            ),
        };
        let accepted = self.submit(intent)?;
        if accepted {
            self.received_signals += 1;
            if self.received_signals > 66 {
                return Err("remote signal count".into());
            }
        }
        Ok(accepted)
    }
    fn event(&mut self) -> Result<Option<Signal>> {
        use native_runtime::Observation;
        let Some(event) = self
            .media
            .advance(self.call)
            .map_err(|e| format!("native advance {e:?}"))?
        else {
            return Ok(None);
        };
        match event {
            Observation::Ready => {
                self.ready = true;
                self.ready_ms = Some(self.started.elapsed().as_millis());
            }
            Observation::Connected => self.connected = true,
            Observation::LocalDescription(sdp) => {
                return Ok(Some(match sdp.kind() {
                    DescriptionKind::Offer => Signal::Offer(bounded(sdp.as_str())?),
                    DescriptionKind::Answer => Signal::Answer(bounded(sdp.as_str())?),
                }));
            }
            Observation::LocalCandidate(c) => {
                return Ok(Some(Signal::Candidate {
                    candidate: bounded(c.candidate())?,
                    mid: bounded(c.mid())?,
                    index: c.index(),
                }));
            }
            Observation::Counters(c) => self.counters = c,
            Observation::Drained => return Err("native drained during exchange".into()),
            Observation::Failed(code) => return Err(format!("native failure category {code}")),
        }
        Ok(None)
    }
    fn exchanged(&self) -> bool {
        self.connected && self.counters[1] >= 20 && self.counters[3] >= 50 && self.counters[11] >= 1
    }
    fn stop(&mut self) -> Value {
        if let Some(report) = &self.stop_report {
            return report.clone();
        }
        let start = Instant::now();
        self.media.close();
        while self.media.outcome() == Exit::Running && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let report = json!({"elapsed_ms":start.elapsed().as_secs_f64()*1000.0,"outcome":format!("{:?}",self.media.outcome()),"physical_devices":"not opened"});
        self.stop_report = Some(report.clone());
        report
    }
}
type ReadFuture = std::pin::Pin<Box<dyn std::future::Future<Output = DiagnosticReadCompletion>>>;
type WriteFuture = std::pin::Pin<Box<dyn std::future::Future<Output = DiagnosticWriteCompletion>>>;
async fn waiting_read(read: &mut Option<ReadFuture>) -> DiagnosticReadCompletion {
    match read {
        Some(read) => read.await,
        None => std::future::pending().await,
    }
}
async fn waiting_write(write: &mut Option<WriteFuture>) -> DiagnosticWriteCompletion {
    match write {
        Some(write) => write.await,
        None => std::future::pending().await,
    }
}
async fn exchange(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    call: CallScope,
    native: &mut Native,
) -> Result<bool> {
    let peers = client
        .diagnostic_peers(&context(runtime, call.account, 3000), call)
        .await
        .map_err(|e| format!("peers {:?}", e.kind))?;
    if peers.as_slice().len() != 1 {
        return Err("exactly one connected browser required".into());
    }
    let initial = &peers.as_slice()[0];
    let offerer = initial.should_offer();
    native.offerer = Some(offerer);
    client
        .diagnostic_select_peer(&context(runtime, call.account, 3000), initial)
        .map_err(|e| format!("selection {:?}", e.kind))?;
    client
        .diagnostic_start_duplex(&context(runtime, call.account, 3000), call)
        .map_err(|e| format!("duplex {:?}", e.kind))?;
    native.duplex = true;
    let mut read: Option<ReadFuture> = None;
    let mut write: Option<WriteFuture> = None;
    let mut page: Option<DiagnosticPoll> = None;
    let mut page_index = 0usize;
    let mut peer: Option<DiagnosticPeer> = None;
    let mut offer_started = false;
    let mut last_stats = Instant::now();
    let mut good_since = None;
    let result: Result<bool> = async {
        if offerer {peer=Some(client.diagnostic_take_selected_peer(&context(runtime,call.account,3000),call).map_err(|e|format!("selected handle {:?}",e.kind))?);}
        loop {
            // Local authority/absolute stop is evaluated independently of HTTP.
            if native.started.elapsed()>=Duration::from_secs(29) {return Err("synthetic absolute observation deadline".into());}
            if let Some(current)=&page {
                let view=current.inspect().map_err(|k|format!("guarded page {k:?}"))?;
                if view.selected_peer_present == Some(false) || view.selected_peer_in_call == Some(false) {
                    native.departure_observation = json!({
                        "membership_observed": view.membership_observed,
                        "selected_peer_present": view.selected_peer_present,
                        "selected_peer_in_call": view.selected_peer_in_call,
                        "request_started_ms": view.request_started.millis,
                    });
                    native.remote_departure = true;
                    return if native.expect_remote_leave && native.exchanged() { Ok(offerer) }
                        else { Err("selected session departed; local stop".into()) };
                }
                if page_index<view.signals.len() {
                    let incoming=&view.signals[page_index];let bound=peer.as_ref().ok_or("signal before selected handle")?;
                    if !incoming.peer().same_negotiation(bound) {return Err("other peer or SID".into());}
                    if native.apply(incoming.signal())? {page_index+=1;}
                } else {
                    let more=view.has_more;page=None;page_index=0;
                    if more {page=Some(client.diagnostic_drain_page(&context(runtime,call.account,3000),call).map_err(|e|format!("mailbox drain {:?}",e.kind))?);}
                }
            }
            if read.is_none() && page.is_none() {
                if native.polls>=16 {return Err("synthetic response count exhausted".into());}
                let ticket=client.diagnostic_prepare_read(&context(runtime,call.account,35000),call).map_err(|e|format!("prepare read {:?}",e.kind))?;
                read=Some(Box::pin(ticket.wait()));native.polls+=1;
            }
            // One serialized outbound future; no extra native-signal queue. The
            // existing16-event IPC owner applies overload/priority-stop behavior.
            if write.is_none() && let Some(signal)=native.event()? {
                    let bound=peer.as_ref().ok_or("native signal without selected peer")?;
                    let ticket=client.diagnostic_prepare_send(&context(runtime,call.account,3000),bound,signal).map_err(|e|format!("prepare send {:?}",e.kind))?;
                    write=Some(Box::pin(ticket.wait()));native.sent_signals+=1;
                    if native.sent_signals>66{return Err("local signal count".into());}
                    if read.is_some(){native.writes_while_reading+=1;}
            }
            if native.ready && offerer && !offer_started {offer_started=native.submit(native_runtime::Intent::Offer)?;}
            if last_stats.elapsed()>=Duration::from_millis(200) && native.submit(native_runtime::Intent::Inspect)? {last_stats=Instant::now();}
            if native.exchanged() {
                let since=good_since.get_or_insert_with(Instant::now);
                if !native.expect_remote_leave && since.elapsed()>=Duration::from_millis(1500) {return Ok(offerer);}
            }
            tokio::select! {
                biased;
                finished=waiting_write(&mut write)=>{
                    write=None;
                    client.diagnostic_finish_write(&context(runtime,call.account,3000),call,finished).map_err(|e|format!("send commit {:?}",e.kind))?;
                }
                finished=waiting_read(&mut read)=>{
                    read=None;
                    let completed=client.diagnostic_finish_read(&context(runtime,call.account,3000),call,finished).map_err(|e|format!("poll {:?} stage {:?} counts {:?}; no replay",e.kind,client.diagnostic_poll_stage(),client.diagnostic_row_counts()))?;
                    let view=completed.inspect().map_err(|e|format!("completed page {e:?}"))?;
                    if peer.is_none() && !view.signals.is_empty() {
                        peer=Some(client.diagnostic_take_selected_peer(&context(runtime,call.account,3000),call).map_err(|e|format!("selected answerer handle {:?}",e.kind))?);
                    }
                    page=Some(completed);page_index=0;
                }
                _=tokio::time::sleep(Duration::from_millis(5))=>{}
            }
        }
    }.await;
    // Priority local stop comes before settling HTTP, never waits for a leave.
    client.diagnostic_stop_signaling();
    native.stop();
    drop(page);
    if let Some(pending) = write.take() {
        let finished = pending.await;
        let _ =
            client.diagnostic_finish_write(&context(runtime, call.account, 3000), call, finished);
    }
    if let Some(pending) = read.take() {
        let finished = pending.await;
        native.stopped_read = true;
        let _ =
            client.diagnostic_finish_read(&context(runtime, call.account, 3000), call, finished);
    }
    let leave = match client.diagnostic_prepare_leave(&context(runtime, call.account, 3000), call) {
        Ok(ticket) => {
            let finished = ticket.wait().await;
            client
                .diagnostic_finish_write(&context(runtime, call.account, 3000), call, finished)
                .map(|_| "acknowledged")
        }
        Err(error) => Err(error),
    };
    native.leave = json!(
        leave
            .as_ref()
            .map(|v| *v)
            .map_err(|e| format!("{:?}", e.kind))
    );
    let deactivate = client
        .request(
            context(runtime, call.account, 3000),
            ProtocolRequest::DeactivateConversation(call.conversation),
        )
        .await;
    native.deactivate = json!(
        deactivate
            .as_ref()
            .map(|_| "acknowledged")
            .map_err(|e| format!("{:?}", e.kind))
    );
    result?;
    leave.map_err(|e| format!("leave {:?}", e.kind))?;
    deactivate.map_err(|e| format!("deactivate {:?}", e.kind))?;
    Ok(offerer)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<_> = std::env::args_os().take(6).collect();
    if !(4..=5).contains(&args.len()) || args.get(4).is_some_and(|v| v != "remote-leave") {
        eprintln!("usage: media-talk-bridge PRIVATE_FIXTURE ABS_HELPER REPORT [remote-leave]");
        std::process::exit(64);
    }
    let mut report = json!({"schema":1,"mode":"synthetic-only; bounded duplex internal AV lifecycle", "result":"fail", "production_authority":false});
    let result = run(
        Path::new(&args[1]),
        Path::new(&args[2]),
        &mut report,
        args.len() == 5,
    )
    .await;
    match result {
        Ok(()) => report["result"] = json!("pass"),
        Err(reason) => report["failure"] = json!(reason),
    }
    let passed = report["result"] == "pass";
    if std::fs::write(
        Path::new(&args[3]),
        serde_json::to_vec_pretty(&report).expect("bounded report"),
    )
    .is_err()
    {
        std::process::exit(74);
    }
    println!("{}", json!({"bridge_result":if passed{"pass"}else{"fail"}}));
    std::process::exit(if passed { 0 } else { 1 });
}
async fn run(fixture: &Path, helper: &Path, report: &mut Value, remote_leave: bool) -> Result<()> {
    let mut scope_bytes = [0; 64];
    getrandom::fill(&mut scope_bytes).map_err(|_| "local random identities")?;
    let id = |offset| {
        u128::from_be_bytes(
            scope_bytes[offset..offset + 16]
                .try_into()
                .expect("identity width"),
        )
    };
    let account = AccountScope {
        account: AccountId::from_local(id(0)),
        runtime: RuntimeGeneration::from_local(id(32)),
    };
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(id(48)));
    let mut client = test_support::from_private_fixture(
        fixture,
        account,
        HostId::from_local(id(16)),
        runtime.clone(),
    )
    .map_err(|e| format!("fixture {:?}", e.kind))?;
    let conversation = client
        .fixture_conversation()
        .ok_or("fixture conversation absent")?;
    let call = CallScope {
        account,
        conversation,
        call: CallGeneration::from_local(id(48)),
    };
    let join = client
        .diagnostic_join_synthetic_call(&context(&runtime, account, 3000), call)
        .await;
    // An unsuccessful join may still have created an active room session. Always
    // attempt owned leave/deactivate, and never claim an uncertain write failed.
    let mut duplex_cleanup = false;
    let result = match join {
        Err(e) => Err(format!("join {:?}", e.kind)),
        Ok(()) => {
            println!("{}", json!({"bridge_state":"joined synthetic diagnostic"}));
            match Native::start(helper, Scope(scope_bytes), call) {
                Err(e) => Err(e),
                Ok(mut native) => {
                    native.expect_remote_leave = remote_leave;
                    let result = exchange(&mut client, &runtime, call, &mut native).await;
                    duplex_cleanup = native.duplex;
                    if native.duplex {
                        report["leave"] = native.leave.clone();
                        report["deactivate"] = native.deactivate.clone();
                    }
                    report["remote_departure"] = json!({"expected":remote_leave,"observed":native.remote_departure,"observation":native.departure_observation});
                    report["duplex"] = json!({"enabled":native.duplex,"prepared_reads":native.polls,"writes_while_reading":native.writes_while_reading,"reader_settled_after_stop":native.stopped_read,"additional_signal_queue":0});
                    report["native_ready_ms"] = json!(native.ready_ms);
                    report["native_connected"] = json!(native.connected);
                    report["native_offerer"] = json!(native.offerer);
                    report["remote_profile"] = native.remote_profile.clone();
                    report["counters"] = json!({"video_source_frames":native.counters[0],"video_decoded_frames":native.counters[1],"audio_source_blocks_10ms":native.counters[2],"audio_nonzero_blocks_10ms":native.counters[3],"successful_socket_sends":native.counters[4],"gate_denied_sends":native.counters[5],"last_socket_send_us":native.counters[6],"rejected_decode_frames":native.counters[7],"helper_elapsed_us":native.counters[8]});
                    report["signaling"] = json!({"accepted_for_processing":native.sent_signals,"received":native.received_signals,"acceptance_is_forwarding_proof":false});
                    report["data_transport"] = json!({"channels_created":native.counters[10],"channels_opened":native.counters[11],"channels_closed":native.counters[12],"messages_received":native.counters[13],"bytes_received":native.counters[14],"rejections":native.counters[15],"negotiations_completed":native.counters[16],"signaling_state":native.counters[17]});
                    report["stop"] = native.stop();
                    result.map(|offerer| {
                        report["native_offerer"] = json!(offerer);
                    })
                }
            }
        }
    };
    if duplex_cleanup {
        return result;
    }
    let leave = client
        .request(
            context(&runtime, account, 3000),
            ProtocolRequest::Leave(call),
        )
        .await;
    let deactivate = client
        .request(
            context(&runtime, account, 3000),
            ProtocolRequest::DeactivateConversation(conversation),
        )
        .await;
    report["leave"] = json!(
        leave
            .as_ref()
            .map(|_| "acknowledged")
            .map_err(|e| format!("{:?}", e.kind))
    );
    report["deactivate"] = json!(
        deactivate
            .as_ref()
            .map(|_| "acknowledged")
            .map_err(|e| format!("{:?}", e.kind))
    );
    result?;
    leave.map_err(|e| format!("leave {:?}", e.kind))?;
    deactivate.map_err(|e| format!("deactivate {:?}", e.kind))?;
    Ok(())
}
