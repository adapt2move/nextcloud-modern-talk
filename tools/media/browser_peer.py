#!/usr/bin/env python3
"""Bounded isolated browser/native protocol cases; never Talk interoperability.

Uses the documented VM-local Playwright workflow, ordinary interface gathering
and synthetic AV. SDP, ICE and channel content never reach evidence files.
"""
import argparse
import asyncio
import json
from pathlib import Path
import struct
import time
from playwright.async_api import async_playwright
from interop import Peer, sha
from talk_browser import INSTRUMENT

FLAGS = ["--use-fake-device-for-media-stream", "--use-fake-ui-for-media-stream",
         "--disable-features=WebRtcHideLocalIpsWithMdns", "--autoplay-policy=no-user-gesture-required"]


async def run(args, report):
    peer = None
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(channel="chromium", headless=True, args=FLAGS)
        report["browser_version"] = browser.version
        context = await browser.new_context(permissions=["camera", "microphone"])
        await context.add_init_script("(" + INSTRUMENT + ")()")
        page = await context.new_page()
        await page.route("http://127.0.0.1:8999/", lambda route: route.fulfill(body="<title>Isolated native diagnostic</title>", content_type="text/html"))
        await page.goto("http://127.0.0.1:8999/")
        try:
            peer = Peer(args.driver, args.helper)
            peer.sample()
            start = time.monotonic()
            while peer.ready_ms is None and time.monotonic() - start < 2:
                peer.read()
                await asyncio.sleep(.005)
            if peer.ready_ms is None:
                raise RuntimeError("native-ready")
            offer = await page.evaluate("""async (single) => {
              window.pc=new RTCPeerConnection({iceServers:[]}); window.ice=[];window.iceOverflow=false;
              pc.addEventListener('track',event=>{if(event.track.kind==='audio'){
                window.remoteAudio=new Audio();remoteAudio.srcObject=new MediaStream([event.track]);
                remoteAudio.volume=0;remoteAudio.play();
              }});
              pc.addEventListener('icecandidate', e=>{if(e.candidate){if(ice.length>=64){iceOverflow=true;return;}ice.push(e.candidate.toJSON());}});
              window.stream=await navigator.mediaDevices.getUserMedia({audio:true,video:{width:640,height:360,frameRate:20}});
              for(const track of stream.getTracks())pc.addTrack(track,stream);
              window.statusChannel=pc.createDataChannel('status');
              window.simpleChannel=single?null:pc.createDataChannel('simplewebrtc');
              await pc.setLocalDescription(await pc.createOffer()); return pc.localDescription.sdp;
            }""", args.case == "unordered")
            peer.description(1, offer)
            report['initial_inline_candidates'] = offer.count('a=candidate:')
            del offer
            assembly = bytearray()
            answer_applied = False
            pending = []
            phase = 'initial'
            stimulus = None
            answer_count = 0
            while time.monotonic() - start < 24:
                if await page.evaluate('iceOverflow'):
                    raise RuntimeError('browser-candidate-overflow')
                batch = await page.evaluate("ice.splice(0,4)")
                for candidate in batch:
                    peer.candidate(candidate)
                try:
                    peer.send(6)
                    events = peer.read()
                except (RuntimeError, BrokenPipeError):
                    peer.read(cleanup=True)
                    if stimulus is not None and args.case in ('binary', 'oversize', 'third-channel', 'unordered'):
                        report['native_rejection_observed'] = peer.failure_category in range(24, 32) if peer.failure_category is not None else False
                        report['native_rejection_ms'] = (time.monotonic() - stimulus)*1000
                        report['result'] = 'pass' if report['native_rejection_observed'] else 'fail'
                        break
                    raise
                for kind, _, payload in events:
                    if kind == 101:
                        total, offset, description_kind = struct.unpack_from(">IIB", payload)
                        if not 0 < total <= 65536 or offset != len(assembly) or description_kind != 2:
                            raise RuntimeError("answer-bound")
                        assembly.extend(payload[9:])
                        if len(assembly) > total:
                            raise RuntimeError("answer-overflow")
                        if len(assembly) == total:
                            answer = assembly.decode()
                            if args.case == 'oversize':
                                # Explicit malicious-peer fault: only this isolated
                                # receiver is lied to about the advertised ceiling.
                                answer = answer.replace('a=max-message-size:4096', 'a=max-message-size:65536')
                                report['fault_answer_max_message_advertisement'] = 65536
                            await page.evaluate("sdp=>pc.setRemoteDescription({type:'answer',sdp})", answer)
                            del answer
                            assembly.clear()
                            answer_applied = True
                            answer_count += 1
                            for item in pending:
                                await page.evaluate("c=>pc.addIceCandidate(c)", item)
                            pending.clear()
                    elif kind == 102:
                        length, index = payload[:2]
                        item = {"sdpMid":payload[2:2+length].decode(), "sdpMLineIndex":index,
                                "candidate":payload[2+length:].decode()}
                        if answer_applied:
                            await page.evaluate("c=>pc.addIceCandidate(c)", item)
                        elif len(pending) < 64:
                            pending.append(item)
                        else:
                            raise RuntimeError("candidate-bound")
                peer.sample()
                stats = await page.evaluate("__nmt_stats()")
                report["browser"] = stats
                report["native_counters"] = peer.stats
                report['answer_count'] = answer_count
                media = peer.stats.get("video_decoded_frames", 0) >= 20 and peer.stats.get("audio_nonzero_blocks_10ms", 0) >= 50
                rows = [row for p in stats for row in p["rows"]]
                media = media and any(r.get("framesDecoded", 0) >= 20 for r in rows) and any(p["decoded_audio_peak_rms"] > .005 for p in stats)
                opened = await page.evaluate("statusChannel.readyState==='open'&&(!simpleChannel||simpleChannel.readyState==='open')")
                if media and opened and phase == 'initial':
                    report['initial_bidirectional_media'] = True
                    stimulus = time.monotonic()
                    phase = 'stimulated'
                    report['stimulus_attempted'] = args.case
                    if args.case == 'binary':
                        await page.evaluate("statusChannel.send(new Uint8Array([1]))")
                    elif args.case == 'oversize':
                        await page.evaluate("statusChannel.send('x'.repeat(4097))")
                    elif args.case == 'third-channel':
                        await page.evaluate("window.extra=pc.createDataChannel('status')")
                    elif args.case == 'unordered':
                        # Existing reliable status channel and AV proved before
                        # the second channel submits an unordered DCEP profile.
                        await page.evaluate("window.simpleChannel=pc.createDataChannel('simplewebrtc',{ordered:false})")
                    elif args.case == 'renegotiate':
                        offer = await page.evaluate("async()=>{await pc.setLocalDescription(await pc.createOffer());return pc.localDescription.sdp}")
                        report['renegotiation_inline_candidates'] = offer.count('a=candidate:')
                        peer.description(1, offer)
                        del offer
                    else:
                        await page.evaluate("()=>{statusChannel.send('x'.repeat(4096));simpleChannel.send('')}")
                    report['stimulus_api_returned'] = True
                if phase == 'stimulated' and args.case == 'baseline' and peer.stats.get('data_bytes_received', 0) >= 4096 and peer.stats.get('data_messages_received', 0) >= 2:
                    report['result'] = 'pass'
                    break
                if phase == 'stimulated' and args.case == 'renegotiate' and answer_count == 2 and peer.stats.get('negotiations_completed', 0) == 2 and media:
                    report['result'] = 'pass'
                    break
                await asyncio.sleep(.025)
        except Exception as error:
            report['failure_type'] = type(error).__name__
        finally:
            if peer:
                report["stop"] = peer.stop()
                report['final_native_counters'] = peer.stats
                if report.get('native_rejection_ms', 10000) >= 1000 and args.case in ('binary', 'oversize', 'third-channel', 'unordered'):
                    report['result'] = 'fail'
            await context.close()
            await browser.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ("driver", "helper", "output"):
        parser.add_argument("--" + key, type=Path, required=True)
    parser.add_argument('--case', choices=['baseline', 'renegotiate', 'binary', 'oversize', 'third-channel', 'unordered'], default='baseline')
    args = parser.parse_args()
    report = {"schema":2,"result":"fail","workflow":"isolated-browser-offerer",'case':args.case,
              "talk_interoperability":False,"physical_devices":False,
              "browser_network_profile":"ordinary interfaces; fake AV; mDNS hiding disabled",'browser_flags':FLAGS, 'browser_candidate_drain_batch':4,
              "harness_sha256":sha(__file__), "observer_sha256":sha(Path(__file__).with_name("talk_browser.py")),
              "driver_sha256":sha(args.driver),"helper_sha256":sha(args.helper)}
    try:
        asyncio.run(run(args, report))
    except Exception as error:
        report["failure_type"] = type(error).__name__
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"result":report["result"]}))
    raise SystemExit(0 if report["result"] == "pass" else 1)


if __name__ == "__main__":
    main()
