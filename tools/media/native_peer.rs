//! Finite engine qualification harness. Raw generated samples are explicit here;
//! normal calls use the same engine with separately admitted physical sources.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/engine.rs"]
mod engine;

use engine::{
    CandidateKind, Event, IcePolicy, IceServer, NativePeer, PeerConnectionState, RtcConfiguration,
    VideoFrame,
};
use nextcloud_talk_api::signaling::{DescriptionKind, IceCandidate, SessionDescription};
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    sync::mpsc,
    time::{Duration, Instant},
};

fn write(value: Value) -> Result<(), String> {
    let mut out = io::stdout().lock();
    writeln!(out, "{value}").map_err(|_| "output closed".to_owned())?;
    out.flush().map_err(|_| "output closed".to_owned())
}

fn configuration() -> Result<RtcConfiguration, String> {
    let args = std::env::args().collect::<Vec<_>>();
    let Some(index) = args.iter().position(|s| s == "--ice-fixture") else {
        return Ok(RtcConfiguration::default());
    };
    let path = args.get(index + 1).ok_or("ICE fixture path")?;
    let file = std::fs::File::open(path).map_err(|_| "ICE fixture open")?;
    let meta = file.metadata().map_err(|_| "ICE fixture metadata")?;
    if !meta.is_file() || meta.len() > 16_384 {
        return Err("ICE fixture bounds".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            return Err("ICE fixture must be private".into());
        }
    }
    let mut bytes = zeroize::Zeroizing::new(Vec::new());
    file.take(16_385)
        .read_to_end(&mut bytes)
        .map_err(|_| "ICE fixture read")?;
    if bytes.len() > 16_384 {
        return Err("ICE fixture bounds".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "ICE fixture JSON")?;
    let ice_policy = match value["policy"].as_str() {
        Some("all") => IcePolicy::All,
        Some("relay") => IcePolicy::Relay,
        _ => return Err("ICE fixture policy".into()),
    };
    let mut ice_servers = Vec::new();
    let servers = value["servers"].as_array().ok_or("ICE fixture servers")?;
    if servers.len() > 8 {
        return Err("ICE fixture servers bound".into());
    }
    for server in servers {
        let urls = server["urls"].as_array().ok_or("ICE fixture URLs")?;
        if urls.is_empty() || urls.len() > 4 {
            return Err("ICE fixture URLs bound".into());
        }
        ice_servers.push(IceServer {
            urls: urls
                .iter()
                .map(|u| u.as_str().map(str::to_owned).ok_or("ICE fixture URL"))
                .collect::<Result<_, _>>()?,
            username: server["username"].as_str().unwrap_or("").into(),
            password: server["credential"].as_str().unwrap_or("").into(),
        });
    }
    Ok(RtcConfiguration {
        ice_servers,
        ice_policy,
        ..Default::default()
    })
}
fn candidate_kind(kind: CandidateKind) -> &'static str {
    match kind {
        CandidateKind::Host => "host",
        CandidateKind::ServerReflexive => "srflx",
        CandidateKind::PeerReflexive => "prflx",
        CandidateKind::Relay => "relay",
        CandidateKind::Unknown => "unknown",
    }
}
async fn run() -> Result<(), String> {
    let offerer = std::env::args().any(|a| a == "--offer");
    let (send, receive) = mpsc::sync_channel::<Value>(16);
    std::thread::Builder::new()
        .name("probe-input".into())
        .spawn(move || {
            let mut input = io::stdin().lock();
            loop {
                let mut line = Vec::new();
                match input.by_ref().take(70_000).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() >= 70_000 => break,
                    Ok(_) => {}
                }
                let Ok(value) = serde_json::from_slice(&line) else {
                    break;
                };
                if send.try_send(value).is_err() {
                    break;
                }
            }
        })
        .map_err(|_| "input worker unavailable")?;
    let config = configuration()?;
    let relay_required = config.ice_policy == IcePolicy::Relay;
    let mut peer = NativePeer::new(config)
        .await
        .map_err(|e| format!("start {e:?}"))?;
    peer.receive_video(true);
    peer.publish(true, true)
        .map_err(|e| format!("publish {e:?}"))?;
    if offerer {
        let offer = peer.offer().await.map_err(|e| format!("offer {e:?}"))?;
        write(json!({"type":"offer","sdp":offer.as_str()}))?;
    }
    let start = Instant::now();
    let mut tick = tokio::time::interval(Duration::from_millis(10));
    let mut frame = 0u64;
    let mut connected = false;
    let mut status_sent = false;
    let mut audio_nonzero = 0u64;
    let mut remote_ready = false;
    let mut candidates = Vec::with_capacity(64);
    loop {
        tick.tick().await;
        if start.elapsed() > Duration::from_secs(25) {
            break;
        }
        if let Ok(message) = receive.try_recv() {
            match message["type"].as_str() {
                Some("offer" | "answer") => {
                    let offer = message["type"] == "offer";
                    let sdp = message["sdp"].as_str().ok_or("missing SDP")?;
                    let value = SessionDescription::new(
                        if offer {
                            DescriptionKind::Offer
                        } else {
                            DescriptionKind::Answer
                        },
                        sdp,
                    )
                    .map_err(|_| "bad SDP")?;
                    if let Some(answer) = peer
                        .remote(&value)
                        .await
                        .map_err(|e| format!("remote {e:?}"))?
                    {
                        write(json!({"type":"answer","sdp":answer.as_str()}))?;
                    }
                    remote_ready = true;
                    for candidate in candidates.drain(..) {
                        peer.candidate(&candidate)
                            .await
                            .map_err(|e| format!("candidate {e:?}"))?;
                    }
                }
                Some("candidate") => {
                    let mid = message["mid"].as_str().ok_or("candidate mid")?;
                    let index = message["index"]
                        .as_u64()
                        .and_then(|n| u8::try_from(n).ok())
                        .ok_or("candidate index")?;
                    let candidate = IceCandidate::new(
                        mid,
                        index,
                        message["candidate"].as_str().ok_or("candidate")?,
                    )
                    .map_err(|_| "bad candidate")?;
                    if remote_ready {
                        peer.candidate(&candidate)
                            .await
                            .map_err(|e| format!("candidate {e:?}"))?;
                    } else if candidates.len() < 64 {
                        candidates.push(candidate);
                    } else {
                        return Err("candidate overflow".into());
                    }
                }
                Some("stop") => break,
                _ => return Err("unknown input".into()),
            }
        }
        while let Some(event) = peer.event().await.map_err(|e| format!("event {e:?}"))? {
            match event {
                Event::Candidate(candidate) => write(
                    json!({"type":"candidate","mid":candidate.mid(),"index":candidate.index(),"candidate":candidate.candidate()}),
                )?,
                Event::Connection(state) => {
                    connected = state == PeerConnectionState::Connected;
                    write(json!({"type":"connection","connected":connected}))?;
                }
            }
        }
        let mut samples = [0i16; 480];
        for (index, sample) in samples.iter_mut().enumerate() {
            let phase =
                (frame * 480 + index as u64) as f64 * 440. * std::f64::consts::TAU / 48_000.;
            *sample = (phase.sin() * 6000.) as i16;
        }
        if connected {
            peer.audio(&samples)
                .await
                .map_err(|e| format!("audio {e:?}"))?;
        }
        if connected && frame.is_multiple_of(5) {
            let mut rgba = vec![0; 640 * 480 * 4];
            for pixel in rgba.as_chunks_mut::<4>().0 {
                pixel.copy_from_slice(&[(frame % 200) as u8, 80, 180, 255]);
            }
            let image = VideoFrame {
                width: 640,
                height: 480,
                rgba,
            };
            peer.video(image, start.elapsed().as_micros() as i64)
                .map_err(|e| format!("video {e:?}"))?;
        }
        while let Some(audio) = peer.take_audio() {
            if audio.data.iter().any(|s| s.unsigned_abs() > 100) {
                audio_nonzero += 1;
            }
        }
        while peer.take_video().is_some() {}
        if connected && !status_sent && peer.send_status(br#"{"type":"audioOn"}"#).await.is_ok() {
            status_sent = true;
        }
        frame += 1;
    }
    let evidence = tokio::time::timeout(Duration::from_secs(2), peer.ice_evidence())
        .await
        .map_err(|_| "selected pair deadline")?
        .map_err(|e| format!("selected pair {e:?}"))?;
    let unsupported = peer
        .ice_support()
        .unsupported
        .iter()
        .map(|s| format!("{s:?}"))
        .collect::<Vec<_>>();
    let selected = evidence.selected.map(
        |s| json!({"local":candidate_kind(s.local),"remote":candidate_kind(s.remote),"udp":s.udp}),
    );
    if relay_required
        && evidence
            .selected
            .is_some_and(|s| s.local != CandidateKind::Relay)
    {
        return Err("relay requirement violated".into());
    }
    peer.close();
    let c = peer.counters();
    write(
        json!({"type":"result","audio_sent":c.audio_sent,"video_sent":c.video_sent,
        "audio_received":c.audio_received,"audio_nonzero":audio_nonzero,"video_received":c.video_received,
        "data_received":c.data_received,"status_sent":status_sent,"closed":true,
        "ice":{"relay_required":relay_required,"host_candidates":evidence.host_candidates,
        "srflx_candidates":evidence.server_reflexive_candidates,"relay_candidates":evidence.relay_candidates,
        "selected":selected,"unsupported":unsupported}}),
    )
}
fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    if let Err(reason) = runtime.block_on(run()) {
        eprintln!("native-probe: {reason}");
        std::process::exit(1);
    }
}
