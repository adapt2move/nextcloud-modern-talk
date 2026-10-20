//! Finite engine qualification harness. Raw generated samples are explicit here;
//! normal calls use the same engine with separately admitted physical sources.
#![forbid(unsafe_code)]
#[path = "../../crates/talk-adapters/src/media/engine.rs"]
mod engine;

use engine::{Event, NativePeer};
use libwebrtc::{peer_connection_factory::RtcConfiguration, video_frame::I420Buffer};
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
    let mut peer =
        NativePeer::new(RtcConfiguration::default()).map_err(|e| format!("start {e:?}"))?;
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
        while let Some(event) = peer.event().map_err(|e| format!("event {e:?}"))? {
            match event {
                Event::Candidate(candidate) => write(
                    json!({"type":"candidate","mid":candidate.mid(),"index":candidate.index(),"candidate":candidate.candidate()}),
                )?,
                Event::Connection(state) => {
                    connected = state == libwebrtc::peer_connection::PeerConnectionState::Connected;
                    write(json!({"type":"connection","connected":connected}))?;
                }
                _ => {}
            }
        }
        let mut samples = [0i16; 480];
        for (index, sample) in samples.iter_mut().enumerate() {
            let phase =
                (frame * 480 + index as u64) as f64 * 440. * std::f64::consts::TAU / 48_000.;
            *sample = (phase.sin() * 6000.) as i16;
        }
        peer.audio(&samples)
            .await
            .map_err(|e| format!("audio {e:?}"))?;
        if frame.is_multiple_of(5) {
            let mut image = I420Buffer::new(640, 480);
            let (y, u, v) = image.data_mut();
            for (index, p) in y.iter_mut().enumerate() {
                *p = ((index as u64 + frame) % 180 + 32) as u8;
            }
            u.fill(96);
            v.fill(160);
            peer.video(image, start.elapsed().as_micros() as i64)
                .map_err(|e| format!("video {e:?}"))?;
        }
        while peer.take_audio().is_some() {}
        while peer.take_video().is_some() {}
        if connected && !status_sent && peer.send_status(br#"{"type":"audioOn"}"#).is_ok() {
            status_sent = true;
        }
        frame += 1;
    }
    peer.close();
    let c = peer.counters();
    write(
        json!({"type":"result","audio_sent":c.audio_sent,"video_sent":c.video_sent,
        "audio_received":c.audio_received,"audio_nonzero":c.audio_nonzero,"video_received":c.video_received,
        "data_received":c.data_received,"status_sent":status_sent,"closed":true}),
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
