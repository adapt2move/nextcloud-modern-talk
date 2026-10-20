#!/usr/bin/env python3
"""Finite maintained-engine interoperability probe; never authorizes physical capture."""
import argparse
import asyncio
import json
from pathlib import Path
from playwright.async_api import async_playwright

SCRIPT = r"""async ({nativeOffers}) => {
  window.nmt = {pc: new RTCPeerConnection(), frames: 0, data: 0, audio_nonzero: 0, received: 0, pending: [], channels: []};
  const n = window.nmt, pc = n.pc;
  pc.onicecandidate = e => { if (e.candidate) nmtSignal({type:'candidate', mid:e.candidate.sdpMid, index:e.candidate.sdpMLineIndex, candidate:e.candidate.candidate}); };
  pc.ontrack = e => {
    n.received++;
    const element = document.createElement(e.track.kind === 'video' ? 'video' : 'audio');
    element.autoplay = true; element.muted = true;
    element.srcObject = new MediaStream([e.track]); document.body.append(element);
    if (e.track.kind === 'video') {
      const count = () => { n.frames++; if (pc.connectionState !== 'closed') element.requestVideoFrameCallback(count); };
      element.requestVideoFrameCallback(count);
    }
    if (e.track.kind === 'audio') {
      const ac = new AudioContext(); ac.resume(); const source=ac.createMediaStreamSource(new MediaStream([e.track])); const analyser=ac.createAnalyser(); const mute=ac.createGain();mute.gain.value=0;source.connect(analyser).connect(mute).connect(ac.destination);const values=new Float32Array(analyser.fftSize);n.audioMeter=setInterval(()=>{analyser.getFloatTimeDomainData(values);if(values.some(v=>Math.abs(v)>0.003))n.audio_nonzero++;},50);n.remoteAudio=ac;
    }
    element.play().catch(() => {});
  };
  const channel = c => { n.channels.push(c); c.onmessage = () => { n.data++; if(n.data===1)c.send(JSON.stringify({type:"ack"})); }; c.onopen = () => c.send(JSON.stringify({type:'nickChanged',payload:{name:'bounded-test'}})); };
  pc.ondatachannel = e => channel(e.channel);
  if (nativeOffers) pc.addEventListener('connectionstatechange',()=>{if(pc.connectionState==='connected'&&!n.statusCreated){n.statusCreated=true;channel(pc.createDataChannel('status'));}});
  if (!nativeOffers) channel(pc.createDataChannel('status'));
  const ac = new AudioContext({sampleRate:48000}); await ac.resume();
  const osc = ac.createOscillator(); osc.frequency.value = 660;
  const gain = ac.createGain(); gain.gain.value = 0.08;
  const dest = ac.createMediaStreamDestination(); osc.connect(gain).connect(dest); osc.start();
  n.audio = ac; n.osc = osc;
  const canvas = document.createElement('canvas'); canvas.width=640; canvas.height=480;
  const ctx = canvas.getContext('2d'); let step=0;
  n.timer = setInterval(() => { ctx.fillStyle=`hsl(${step++%360} 80% 50%)`; ctx.fillRect(0,0,640,480); ctx.fillStyle='white'; ctx.fillRect(step%500,120,100,120); },50);
  const stream = canvas.captureStream(20); n.tracks=[...dest.stream.getTracks(),...stream.getTracks()];
  for (const track of n.tracks) pc.addTrack(track,new MediaStream([track]));
  window.nmtReceive = async m => {
    if (m.type === 'candidate') {
      const c={candidate:m.candidate,sdpMid:m.mid,sdpMLineIndex:m.index};
      if (pc.remoteDescription) await pc.addIceCandidate(c); else { if(n.pending.length>=64) throw Error('candidate bound'); n.pending.push(c); }
    } else if (m.type === 'offer' || m.type === 'answer') {
      await pc.setRemoteDescription({type:m.type,sdp:m.sdp});
      for (const c of n.pending.splice(0)) await pc.addIceCandidate(c);
      if (m.type === 'offer') { await pc.setLocalDescription(await pc.createAnswer()); await nmtSignal({type:'answer',sdp:pc.localDescription.sdp}); }
    }
  };
  if (!nativeOffers) { await pc.setLocalDescription(await pc.createOffer()); await nmtSignal({type:'offer',sdp:pc.localDescription.sdp}); }
}"""

async def probe(executable, offer):
    child = await asyncio.create_subprocess_exec(str(executable), *(['--offer'] if offer else []), stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.PIPE, limit=140000)
    summary = {'direction':'native-offer' if offer else 'browser-offer'}
    try:
      async with async_playwright() as p:
        browser=await p.chromium.launch(headless=True,args=['--no-sandbox','--autoplay-policy=no-user-gesture-required'])
        page=await browser.new_page()
        async def send(value):
          child.stdin.write((json.dumps(value)+'\n').encode()); await child.stdin.drain()
        await page.expose_function('nmtSignal',send)
        await page.goto('about:blank')
        await page.evaluate(SCRIPT,{'nativeOffers':offer})
        async def read():
          while line:=await child.stdout.readline():
            if len(line)>70000: raise ValueError('output bound')
            event=json.loads(line)
            if event['type'] in ('offer','answer','candidate'): await page.evaluate('m => nmtReceive(m)',event)
            elif event['type']=='result': summary['native']=event
            elif event['type']=='error': raise RuntimeError(event.get('error','native failed'))
        reader=asyncio.create_task(read())
        await page.wait_for_function("nmt.pc.connectionState === 'connected'",timeout=15000)
        await asyncio.sleep(6)
        summary['browser']=await page.evaluate("""async () => {
          const stats=await nmt.pc.getStats(), received=[], sent=[];
          stats.forEach(s=>{if(s.type==='inbound-rtp')received.push({kind:s.kind,bytes:s.bytesReceived,frames:s.framesDecoded||0,samples:s.totalSamplesReceived||0,energy:s.totalAudioEnergy||0});if(s.type==='outbound-rtp')sent.push({kind:s.kind,bytes:s.bytesSent,frames:s.framesEncoded||0});});
          return {state:nmt.pc.connectionState,video_callbacks:nmt.frames,audio_nonzero:nmt.audio_nonzero,data:nmt.data,received,sent};
        }""")
        await send({'type':'stop'})
        await asyncio.wait_for(reader,5)
        await asyncio.wait_for(child.wait(),5)
        summary['exit']=child.returncode
        await page.evaluate('() => { clearInterval(nmt.timer); for(const t of nmt.tracks)t.stop();nmt.osc.stop();nmt.audio.close();nmt.pc.close(); }')
        await browser.close()
      n,b=summary.get('native',{}),summary['browser']
      summary['passed']=bool(summary['exit']==0 and n.get('closed') and n.get('audio_received',0)>100 and n.get('audio_nonzero',0)>0 and n.get('video_received',0)>10 and n.get('data_received',0)>0 and b['video_callbacks']>10 and b['data']>0 and b['audio_nonzero']>10 and any(s['kind']=='video' and s['frames']>10 for s in b['received']))
    except Exception as e:
      summary['passed']=False
      summary['failure_type']=type(e).__name__
      summary['failure']=str(e)[:200]
    finally:
      if 'reader' in locals():
        if not reader.done(): reader.cancel()
        await asyncio.gather(reader, return_exceptions=True)
      if child.returncode is None: child.kill(); await child.wait()
      stderr=await child.stderr.read(8192)
      summary['stderr_bytes']=len(stderr)
      if stderr: summary['native_error']=stderr.decode(errors='replace')[:500]
      summary['exit']=child.returncode
    return summary

async def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--native',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    results=[]
    for offer in [False,True]:
      try: results.append(await asyncio.wait_for(probe(args.native,offer),35))
      except Exception as e: results.append({'direction':'native-offer' if offer else 'browser-offer','passed':False,'failure_type':type(e).__name__,'failure':str(e)[:200]})
    args.output.write_text(json.dumps(results,indent=2)+'\n')
    print(json.dumps(results))
    if not all(r['passed'] for r in results): raise SystemExit(1)

if __name__=='__main__': asyncio.run(main())
