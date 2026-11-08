//! Bounded synthetic-device exercise of the normal account-owned call path.
//! Only credential intake and generated raw frames are fixture-specific.
#![forbid(unsafe_code)]
#[cfg(target_os = "linux")]
#[path = "../../apps/desktop/src/devices/screen.rs"]
mod screen_capture;
use serde_json::{Value, json};
use std::{
    path::Path,
    time::{Duration, Instant},
};
use talk_adapters::{
    media::MediaService,
    nextcloud::{NetworkRuntime, NextcloudClient, test_support},
};
use talk_core::{
    calls::{Command, Entry, MediaWishes, Phase, Snapshot},
    ids::{AccountId, AccountScope, ClockEpoch, HostId, RuntimeGeneration},
    lifecycle::Cancellation,
    ports::{NextcloudPort, OperationContext, ProtocolReply, ProtocolRequest},
};
fn context(runtime: &NetworkRuntime, scope: AccountScope) -> OperationContext {
    let mut deadline = runtime.now();
    deadline.millis += 3000;
    OperationContext {
        scope,
        deadline,
        cancellation: Cancellation::default(),
    }
}
async fn request(
    client: &mut NextcloudClient,
    runtime: &NetworkRuntime,
    scope: AccountScope,
    request: ProtocolRequest,
) -> Result<ProtocolReply, String> {
    client
        .request(context(runtime, scope), request)
        .await
        .map_err(|e| format!("{:?}", e.kind))
}
fn snapshot(reply: ProtocolReply) -> Result<Snapshot, String> {
    if let ProtocolReply::Meeting(value) = reply {
        Ok(value)
    } else {
        Err("meeting reply type".into())
    }
}
async fn run(fixture: &Path, output: &Path, mode: &str, report: &mut Value) -> Result<(), String> {
    let scope = AccountScope {
        account: AccountId::from_local(1),
        runtime: RuntimeGeneration::from_local(1),
    };
    let runtime = NetworkRuntime::new(ClockEpoch::from_local(1));
    let media = MediaService::new();
    let mut client =
        test_support::from_private_fixture(fixture, scope, HostId::from_local(1), runtime.clone())
            .map_err(|e| format!("intake {:?}", e.kind))?;
    client.fixture_media(media.clone());
    report["stage"] = json!("inspect");
    request(
        &mut client,
        &runtime,
        scope,
        ProtocolRequest::InspectAccount,
    )
    .await?;
    let conversation = client.fixture_conversation().ok_or("fixture room")?;
    report["stage"] = json!("reconcile");
    let room = request(
        &mut client,
        &runtime,
        scope,
        ProtocolRequest::ReconcileConversation(conversation),
    )
    .await?;
    let ProtocolReply::Conversation(room) = room else {
        return Err("room reply".into());
    };
    let mut call = room.call_scope.ok_or("normal call binding")?;
    let mut entry = Entry::PreparedConference;
    if mode.starts_with("incoming-") {
        report["stage"] = json!("incoming source");
        let mut incoming = None;
        for _ in 0..4 {
            if let ProtocolReply::IncomingCalls(value) = request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::PollIncomingCalls,
            )
            .await?
            {
                report["incoming_alert_eligible"] = json!(value.eligible);
                if let Some(value) = value.call {
                    incoming = Some(value);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        let invitation = incoming.ok_or("no current direct invitation")?;
        call = invitation.call;
        entry = Entry::IncomingDirect;
        report["incoming_observed"] = json!(invitation.phase == Phase::Ringing);
        report["incoming_no_capture"] = json!(media.capture().is_none());
        if mode == "incoming-decline" {
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::Stop(call)),
            )
            .await?;
            let reply = request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::PollIncomingCalls,
            )
            .await?;
            report["decline_removed_invitation"] =
                json!(matches!(reply,ProtocolReply::IncomingCalls(value) if value.call.is_none()));
            let participants = request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Participants(conversation),
            )
            .await?;
            report["caller_remains_in_call"] = json!(
                matches!(participants,ProtocolReply::Participants(ref p) if p.people.as_slice().iter().any(|p|!p.is_self&&p.in_call==Some(true)))
            );
            report["publication_closed_synchronously"] = json!(media.capture().is_none());
            client.cancellation().cancel();
            if report["decline_removed_invitation"] == true
                && report["caller_remains_in_call"] == true
                && report["incoming_no_capture"] == true
            {
                return Ok(());
            }
            return Err("decline observation incomplete".into());
        }
    }
    report["stage"] = json!("start");
    let mut current = snapshot(
        request(
            &mut client,
            &runtime,
            scope,
            ProtocolRequest::Meeting(Command::Start {
                call,
                entry,
                wishes: MediaWishes {
                    microphone: mode != "incoming-accept",
                    camera: mode != "incoming-accept",
                    screen: false,
                },
            }),
        )
        .await?,
    )?;
    report["initial_phase"] = json!(format!("{:?}", current.phase));
    let started = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut next_poll = Instant::now();
    let mut frames = 0u64;
    #[cfg(target_os = "linux")]
    let mut portal_capture = None;
    let mut screen_started = false;
    let mut screen_stopped = false;
    let mut screen_seen = false;
    let mut local_screen_flowed = false;
    let mut screen_dimensions = None;
    let mut screen_sent = 0u64;
    let mut stale_screen = None;
    let mut phase = 0u64;
    let mut observed = Vec::new();
    let mut observed_generation = 1u8;
    let mut raw_audio = 0u64;
    let mut raw_video = 0u64;
    let mut played = 0u64;
    let mut drawn = 0u64;
    let mut pixels = vec![0u8; 640 * 480 * 4];
    let mut previous = None;
    let mut reconnected = false;
    let mut resumed = false;
    let mut muted = false;
    let mut unmuted = false;
    let mut counters_before_mute = None;
    let mut publication_after_mute = None;
    let mut observed_before_reconnect = None;
    let mut incoming_enabled = false;
    let continuity = matches!(mode, "network-resume" | "session-replaced");
    let recovery_probe =
        continuity || std::env::var("NMT_MEDIA_RECOVERY_PROBE").as_deref() == Ok("true");
    let mut ready_at: Option<Duration> = None;
    let mute_at = if continuity { 4 } else { 10 };
    let unmute_at = if continuity { 16 } else { 12 };
    // A severed internal poll may remain active server-side for its documented
    // 30-second duration. Exercise recovery after that real response, without
    // treating a fresh REST request as a successful destructive signaling read.
    let recovery_window = std::env::var("NMT_MEDIA_RECOVERY_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| matches!(*v, 32 | 50))
        .unwrap_or(32);
    let limit = if mode == "reconnect" {
        44
    } else if recovery_probe {
        recovery_window
    } else if mode == "web-screen" && recovery_window == 50 {
        50
    } else {
        24
    };
    report["stage"] = json!("exchange");
    report["exchange_window_seconds"] = json!(limit);
    while started.elapsed()
        < if recovery_probe {
            ready_at.map_or(Duration::from_secs(20), |ready| {
                ready + Duration::from_secs(limit)
            })
        } else {
            Duration::from_secs(limit)
        }
    {
        tick.tick().await;
        let control_elapsed = if recovery_probe {
            ready_at
                .map(|ready| started.elapsed().saturating_sub(ready))
                .unwrap_or_default()
        } else {
            started.elapsed()
        };
        if previous.is_none() {
            previous = media.capture();
        }
        if mode == "incoming-accept" && !incoming_enabled && current.phase == Phase::Active {
            report["accepted_receive_only_before_toggle"] = json!(
                media
                    .capture()
                    .is_some_and(|c| c.requested() == MediaWishes::default())
            );
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::Change {
                    call,
                    wishes: MediaWishes {
                        microphone: true,
                        camera: true,
                        screen: false,
                    },
                }),
            )
            .await?;
            incoming_enabled = true;
        }
        if (mode == "reconnect" || continuity)
            && (!recovery_probe || ready_at.is_some())
            && control_elapsed >= Duration::from_secs(mute_at)
            && !muted
        {
            current = snapshot(
                request(
                    &mut client,
                    &runtime,
                    scope,
                    ProtocolRequest::Meeting(Command::Change {
                        call,
                        wishes: MediaWishes::default(),
                    }),
                )
                .await?,
            )?;
            publication_after_mute = Some(
                previous
                    .as_ref()
                    .is_some_and(|c| !c.requested().microphone && !c.requested().camera),
            );
            counters_before_mute = Some(current.counters);
            muted = true;
        }
        if (mode == "reconnect" || continuity)
            && (!recovery_probe || ready_at.is_some())
            && control_elapsed >= Duration::from_secs(unmute_at)
            && !unmuted
        {
            current = snapshot(
                request(
                    &mut client,
                    &runtime,
                    scope,
                    ProtocolRequest::Meeting(Command::Observe(call)),
                )
                .await?,
            )?;
            call = current.call;
            if continuity {
                report["muted_intent_retained"] =
                    json!(!current.wishes.microphone && !current.wishes.camera);
                report["muted_capture_retained"] = json!(
                    media
                        .capture()
                        .is_some_and(|c| !c.requested().microphone && !c.requested().camera)
                );
            }
            report["muted_counters_stopped"] =
                json!(
                    counters_before_mute.is_some_and(|before| current.counters.audio_sent
                        <= before.audio_sent + 1
                        && current.counters.video_sent <= before.video_sent + 1)
                );
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::Change {
                    call,
                    wishes: MediaWishes {
                        microphone: true,
                        camera: true,
                        screen: false,
                    },
                }),
            )
            .await?;
            unmuted = true;
        }
        if mode == "reconnect" && started.elapsed() >= Duration::from_secs(18) && !reconnected {
            observed_before_reconnect = Some(current.counters);
            let old = call;
            current = snapshot(
                request(
                    &mut client,
                    &runtime,
                    scope,
                    ProtocolRequest::Meeting(Command::Reconnect(call)),
                )
                .await?,
            )?;
            report["old_capture_closed"] =
                json!(previous.as_ref().is_some_and(|c| c.scope().is_none()
                    && !c.microphone(&[1; 480])
                    && !c.camera_rgba(640, 480, &pixels)));
            let until = Instant::now() + Duration::from_secs(7);
            while current.call == old && Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(200)).await;
                current = snapshot(
                    request(
                        &mut client,
                        &runtime,
                        scope,
                        ProtocolRequest::Meeting(Command::Observe(old)),
                    )
                    .await?,
                )?;
            }
            call = current.call;
            report["new_call_generation"] = json!(call != old);
            report["reconnect_capture_stays_off"] = json!(
                media
                    .capture()
                    .is_some_and(|c| c.requested() == MediaWishes::default())
            );
            reconnected = true;
        }
        if mode == "reconnect" && started.elapsed() >= Duration::from_secs(28) && !resumed {
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::Change {
                    call,
                    wishes: MediaWishes {
                        microphone: true,
                        camera: true,
                        screen: false,
                    },
                }),
            )
            .await?;
            resumed = true;
        }
        if matches!(
            mode,
            "native-screen" | "native-static-screen" | "portal-screen"
        ) && started.elapsed() > Duration::from_secs(5)
            && !screen_started
        {
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::StartScreenShare(call)),
            )
            .await?;
            screen_started = true;
        }
        if matches!(
            mode,
            "native-screen" | "native-static-screen" | "portal-screen"
        ) && started.elapsed() > Duration::from_secs(15)
            && !screen_stopped
        {
            request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::StopScreenShare(call)),
            )
            .await?;
            screen_stopped = true;
        }
        if let Some(capture) = media.capture() {
            let wishes = capture.requested();
            if wishes.microphone {
                let mut samples = [0i16; 480];
                for sample in &mut samples {
                    *sample = ((phase as f64 * 440.0 * std::f64::consts::TAU / 48000.0).sin()
                        * 8000.0) as i16;
                    phase += 1;
                }
                raw_audio += u64::from(capture.microphone(&samples));
            }
            if wishes.camera && frames.is_multiple_of(5) {
                for (y, row) in pixels
                    .as_chunks_mut::<{ 640 * 4 }>()
                    .0
                    .iter_mut()
                    .enumerate()
                {
                    for (x, p) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                        p.copy_from_slice(&[
                            ((x + frames as usize) % 256) as u8,
                            ((y + frames as usize) % 256) as u8,
                            96,
                            255,
                        ]);
                    }
                }
                raw_video += u64::from(capture.camera_rgba(640, 480, &pixels));
            }
            #[cfg(target_os = "linux")]
            if mode == "portal-screen" {
                if let Some(generation) = capture.screen_generation() {
                    if portal_capture
                        .as_ref()
                        .is_some_and(screen_capture::Capture::finished)
                    {
                        return Err("portal capture ended".into());
                    }
                    if portal_capture.is_none() {
                        portal_capture = Some(
                            screen_capture::Capture::start(capture.clone(), generation)
                                .map_err(|_| "portal worker unavailable")?,
                        );
                    }
                } else {
                    portal_capture = None;
                }
            }
            if (mode == "native-screen" || mode == "native-static-screen" && screen_sent == 0)
                && let Some(generation) = capture.screen_generation()
            {
                stale_screen = Some((capture.clone(), generation));
                if frames.is_multiple_of(10) {
                    let pixels = vec![200; 1280 * 720 * 4];
                    screen_sent += u64::from(capture.screen_rgba(generation, 1280, 720, &pixels));
                }
            }
            let mut out = [0; 480];
            if capture.output_audio(&mut out) && out.iter().any(|s| *s != 0) {
                played += 1;
            }
            if capture.take_video().is_some() {
                drawn += 1;
            }
        }
        local_screen_flowed |= current.sharing == talk_core::calls::SharingState::Sharing;
        if let talk_core::calls::SharingState::Failed(reason) = current.sharing {
            report["capture_failure"] = json!(format!("{:?}", reason));
        }
        for participant in current.participants.as_slice() {
            if participant.kind == talk_core::calls::StreamKind::Screen && participant.flowing {
                screen_seen = true;
                screen_dimensions = participant.dimensions;
            }
        }
        frames += 1;
        if Instant::now() >= next_poll {
            let observed_reply = request(
                &mut client,
                &runtime,
                scope,
                ProtocolRequest::Meeting(Command::Observe(call)),
            )
            .await;
            report["samples"] = json!(&observed);
            current = match observed_reply {
                Ok(value) => snapshot(value)?,
                Err(error)
                    if mode == "policy-loss"
                        && error == "PermissionDenied"
                        && media.capture().is_none() =>
                {
                    report["terminal_observe_error"] = json!(error);
                    break;
                }
                Err(error) => return Err(error),
            };
            if call != current.call {
                observed_generation = observed_generation.saturating_add(1);
                report["automatic_new_call_generation"] = json!(true);
                report["old_capture_closed"] = json!(
                    previous
                        .as_ref()
                        .is_some_and(|c| c.scope().is_none() && !c.microphone(&[1; 480]))
                );
                call = current.call;
            }
            if ready_at.is_none()
                && current.phase == Phase::Active
                && current.counters.audio_received > 50
                && current.counters.video_received > 5
                && current.counters.audio_sent > 50
                && current.counters.video_sent > 5
            {
                ready_at = Some(started.elapsed());
                report["media_ready_ms"] = json!(started.elapsed().as_millis());
            }
            if observed.len() < 96 {
                observed.push(json!({"ms":started.elapsed().as_millis(),"generation":observed_generation,"phase":format!("{:?}",current.phase),"sharing":format!("{:?}",current.sharing),"microphone_intent":current.wishes.microphone,"camera_intent":current.wishes.camera,"screen_flowing":current.participants.as_slice().iter().any(|p|p.kind==talk_core::calls::StreamKind::Screen && p.flowing),"peers":current.peers,"audio_sent":current.counters.audio_sent,"audio_received":current.counters.audio_received,"video_sent":current.counters.video_sent,"video_received":current.counters.video_received,"data_received":current.counters.data_received,"offers_sent":current.counters.offers_sent,"answers_sent":current.counters.answers_sent,"policy_refreshes":current.counters.policy_refreshes}));
            }
            report["samples"] = json!(&observed);
            // Bounded metadata progress lets the browser driver cut transport
            // only after an established call, never during initial negotiation.
            let bytes = serde_json::to_vec(report).map_err(|_| "progress encoding")?;
            if bytes.len() > 131_072 {
                return Err("progress bound".into());
            }
            let temporary = output.with_extension("progress");
            std::fs::write(&temporary, bytes).map_err(|_| "progress write")?;
            std::fs::rename(temporary, output).map_err(|_| "progress replace")?;
            next_poll = Instant::now() + Duration::from_secs(1);
            // Policy fetching is a connecting state, including before admission.
            // Let the bounded probe observe recovery; it still requires active,
            // decoded media at completion and never supplies missing authority.
            if current.phase == Phase::Ended
                || matches!(current.phase, Phase::Waiting(gate)
                if !matches!(gate, talk_core::calls::Gate::PolicyUnavailable(_)))
            {
                break;
            }
        }
    }
    report["samples"] = json!(observed);
    report["local_sharing_state"] = json!(format!("{:?}", current.sharing));
    report["remote_streams"] = json!(current.participants.as_slice().iter().map(|p| json!({"kind":format!("{:?}",p.kind),"dimensions":p.dimensions,"flowing":p.flowing})).collect::<Vec<_>>());
    report["raw_audio"] = json!(raw_audio);
    report["raw_video"] = json!(raw_video);
    report["decoded_audio_blocks_nonzero"] = json!(played);
    report["decoded_video_frames"] = json!(drawn);
    report["duration_ms"] = json!(started.elapsed().as_millis());
    let exchanged = current.phase == Phase::Active
        && current.counters.data_received > 0
        && current.counters.audio_sent > 100
        && current.counters.audio_received > 100
        && current.counters.video_sent > 20
        && current.counters.video_received > 20
        && played > 20
        && drawn > 20
        && started.elapsed() >= Duration::from_secs(20);
    if mode == "policy-loss" {
        // Talk can deny the call-participants read before the concurrent room
        // refresh exposes readOnly. Both authoritative rejections must close
        // capture. The browser harness separately requires the owned policy
        // mutation after established bidirectional media.
        let rejected = current.phase == Phase::Waiting(talk_core::calls::Gate::RoomRestricted)
            || report["terminal_observe_error"] == "PermissionDenied";
        let revoked = rejected
            && ready_at.is_some()
            && media.capture().is_none()
            && previous
                .as_ref()
                .is_some_and(|c| !c.microphone(&[1; 480]) && !c.camera_rgba(640, 480, &pixels));
        report["policy_loss_closes_capture"] = json!(revoked);
        report["remote_leave"] =
            json!("not confirmed under read-only policy; fixture cleanup follows");
        client.cancellation().cancel();
        if revoked && started.elapsed() < Duration::from_secs(22) && played > 20 && drawn > 20 {
            return Ok(());
        }
        return Err("policy loss observation incomplete".into());
    }
    if mode == "reconnect" {
        report["mute_closed_synchronously"] = json!(publication_after_mute == Some(true));
        report["resumed_after_explicit_intent"] = json!(
            resumed
                && current.counters.audio_sent > 100
                && current.counters.video_sent > 20
                && observed_before_reconnect.is_some_and(|c| c.audio_sent > 100)
        );
    }
    if matches!(mode, "native-screen" | "native-static-screen") {
        report["screen_generated_frames"] = json!(screen_sent);
        report["screen_stopped"] = json!(screen_stopped);
        report["stale_screen_rejected"] = json!(
            stale_screen
                .as_ref()
                .is_some_and(|(h, g)| !h.screen_rgba(*g, 2, 2, &[0; 16]))
        );
        if screen_sent
            < if mode == "native-static-screen" {
                1
            } else {
                20
            }
            || !screen_stopped
            || report["stale_screen_rejected"] != true
        {
            return Err("screen lifecycle incomplete".into());
        }
    }
    if mode == "portal-screen" {
        report["portal_frames_published"] = json!(local_screen_flowed);
        if !local_screen_flowed {
            return Err("portal source never flowed".into());
        }
    }
    if mode == "web-screen" {
        report["screen_received"] = json!(screen_seen);
        report["screen_dimensions"] = json!(screen_dimensions);
        if !screen_seen {
            return Err("remote screen never flowed".into());
        }
    }
    report["stage"] = json!("leave");
    request(
        &mut client,
        &runtime,
        scope,
        ProtocolRequest::Meeting(Command::Stop(call)),
    )
    .await?;
    let closed = media.capture().is_none();
    report["publication_closed_synchronously"] = json!(closed);
    tokio::time::sleep(Duration::from_secs(1)).await;
    let after = request(
        &mut client,
        &runtime,
        scope,
        ProtocolRequest::ReconcileConversation(conversation),
    )
    .await?;
    report["room_retained"] = json!(matches!(after, ProtocolReply::Conversation(_)));
    let participants = request(
        &mut client,
        &runtime,
        scope,
        ProtocolRequest::Participants(conversation),
    )
    .await?;
    report["peer_remains_in_call"] = json!(
        matches!(participants,ProtocolReply::Participants(ref p) if p.people.as_slice().iter().any(|p|!p.is_self&&p.in_call==Some(true)))
    );
    client.cancellation().cancel();
    report["stage"] = json!("closed");
    let lifecycle = mode != "reconnect"
        || [
            "mute_closed_synchronously",
            "muted_counters_stopped",
            "old_capture_closed",
            "new_call_generation",
            "reconnect_capture_stays_off",
            "resumed_after_explicit_intent",
        ]
        .iter()
        .all(|key| report[key] == true);
    let continuity_ok = !continuity
        || report["muted_intent_retained"] == true
            && report["muted_capture_retained"] == true
            && report["muted_counters_stopped"] == true
            && (mode != "session-replaced"
                || report["automatic_new_call_generation"] == true
                    && report["old_capture_closed"] == true);
    if exchanged && closed && lifecycle && continuity_ok && report["peer_remains_in_call"] == true {
        Ok(())
    } else {
        Err("normal media or cleanup observations incomplete".into())
    }
}
fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() != 4
        || ![
            "media",
            "reconnect",
            "policy-loss",
            "incoming-accept",
            "incoming-decline",
            "native-screen",
            "native-static-screen",
            "portal-screen",
            "web-screen",
            "network-resume",
            "session-replaced",
        ]
        .contains(&args[3].as_str())
    {
        std::process::exit(2);
    }
    let mut report = json!({"workflow":"normal-account-call","result":"fail","physical_devices":false,"scenario":args[3]});
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("bounded runtime");
    match runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(92),
            run(
                Path::new(&args[1]),
                Path::new(&args[2]),
                &args[3],
                &mut report,
            ),
        )
        .await
    }) {
        Ok(Ok(())) => report["result"] = json!("pass"),
        Ok(Err(error)) => report["failure"] = json!(error),
        Err(_) => report["failure"] = json!("overall deadline"),
    };
    let ok = report["result"] == "pass";
    std::fs::write(
        &args[2],
        serde_json::to_vec_pretty(&report).expect("report"),
    )
    .expect("write report");
    runtime.shutdown_timeout(Duration::from_secs(2));
    std::process::exit(if ok { 0 } else { 1 });
}
