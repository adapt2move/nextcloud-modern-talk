#!/usr/bin/env python3
"""Finite original Talk browser against the normal production account call path."""
import argparse
import os
from contextlib import AsyncExitStack
import asyncio,base64,json,tempfile,time,urllib.request,ssl
from pathlib import Path
from urllib.parse import urlparse,urlencode
from playwright.async_api import async_playwright
from ice_settings import fetch_settings
from recovery_checks import native_recovery
from talk_browser import LoopbackForward,FreshRoom,INSTRUMENT,press,media_checks,private_json,digest
# Observe original browser SCTP messages without retaining any payloads.
INSTRUMENT = INSTRUMENT.replace("const NativePC = window.RTCPeerConnection;", """const NativePC = window.RTCPeerConnection;
 const Socket=window.WebSocket, socketStates=[];window.__nmt_sockets=socketStates;
 window.WebSocket=new Proxy(Socket,{construct(target,args){const socket=new target(...args);const row=[];if(socketStates.length<8)socketStates.push(row);for(const event of ['open','close','error'])socket.addEventListener(event,()=>{if(row.length<32)row.push(event);});socket.addEventListener('message',event=>{try{const value=JSON.parse(event.data);if(row.length<32)row.push({type:value.type,error:value.error?.code});}catch{}});return socket;}});
""")
INSTRUMENT = INSTRUMENT.replace('states:[]};dc.push(row);','states:[],received:0,sent:0};dc.push(row);').replace(
 "markChannel();for(const event", "channel.addEventListener('message',()=>{row.received=Math.min(100,row.received+1);});const send=channel.send.bind(channel);channel.send=(...args)=>{const result=send(...args);row.sent=Math.min(100,row.sent+1);return result;};markChannel();for(const event")
class FreshDirect(FreshRoom):
 async def owned_matches(self):
  rooms=await asyncio.to_thread(self.request,'GET','?noStatusUpdate=1&includeLastMessage=false')
  if not isinstance(rooms,list) or len(rooms)>32:raise RuntimeError('direct room list bound')
  matches=[r['token'] for r in rooms if r.get('type')==1 and r.get('name')=='lab_alice' and self.valid_token(r.get('token'))]
  if len(matches)>1:raise RuntimeError('direct room ownership ambiguous')
  return matches
 async def open(self):
  if self.state!='not_sent' or await self.owned_matches():raise RuntimeError('direct pair already exists')
  self.state='create_sent_once';self.persist()
  room=await asyncio.to_thread(self.request,'POST','',{'roomType':1,'invite':'lab_alice'})
  if room.get('type')!=1 or not self.valid_token(room.get('token')):raise RuntimeError('direct create shape')
  self.token=room['token'];self.state='create_confirmed';self.persist();return self.token
 def request(self,method,suffix,values=None):
  # Talk creates only the initiating participant until first call/message.
  # The synthetic browser caller must therefore create this direct room.
  if method=='DELETE' and suffix=='/'+str(self.token):suffix+='/participants/self'
  if len(self.requests)>=12:raise RuntimeError('direct setup request budget')
  record={'operation':method,'started_unix':time.time(),'timeout_seconds':3};self.requests.append(record)
  class NoRedirect(urllib.request.HTTPRedirectHandler):
   def redirect_request(self,*_):return None
  authorization=base64.b64encode(('lab_outsider:'+self.password).encode()).decode()
  req=urllib.request.Request(self.origin+'/ocs/v2.php/apps/spreed/api/v4/room'+suffix,data=urlencode(values).encode() if values is not None else None,method=method,headers={'Authorization':'Basic '+authorization,'OCS-APIRequest':'true','Accept':'application/json'})
  with urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect()).open(req,timeout=3) as response:
   raw=response.read(65537)
   if len(raw)>65536:raise RuntimeError('direct setup response bound')
   body=json.loads(raw)['ocs'];record['status']=response.status
   if body['meta']['status']!='ok' or body['meta']['statuscode'] not in (100,200,201):raise RuntimeError('direct setup status')
   return body['data']
async def spawn_native(executable,fixture,output,scenario,recovery_window):
 process=await asyncio.create_subprocess_exec(str(executable),str(fixture),str(output),('web-screen' if scenario=='web-screen-resume' else scenario),stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.PIPE,env={**os.environ,'NMT_MEDIA_RECOVERY_SECONDS':str(recovery_window),'NMT_MEDIA_RECOVERY_PROBE':str(scenario in ('network-resume','session-replaced','web-screen-resume')).lower()})
 errors=bytearray()
 async def drain():
  while block:=await process.stderr.read(4096):
   if len(errors)<16384:errors.extend(block[:16384-len(errors)])
 return process,errors,asyncio.create_task(drain())
async def run(args,report):
 m=private_json(args.manifest);c=private_json(m['credentials_file']);origin=urlparse(m['client_url']);target=urlparse(m['vm_url'])
 if not m['ready'] or origin.hostname!='127.0.0.1' or origin.scheme not in ('http','https'):raise RuntimeError('fixture origin')
 report.update(profile=m['profile'],nextcloud=m['nextcloud'],talk=m['talk'],executable_sha256=digest(args.executable),instrument_dependency_sha256=digest(Path(__file__).with_name('talk_browser.py')),room_cleanup_scope='synthetic initiator membership; remaining direct pair participant removed by lab teardown',participants=args.participants)
 async with AsyncExitStack() as stack:
  tls=None
  if origin.scheme=='https':
   root=Path(m['media_tls']);tls=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);tls.load_cert_chain(root/'cert.pem',root/'key.pem')
  main_forward=await stack.enter_async_context(LoopbackForward(target.hostname,origin.port,ssl_context=tls))
  hpb=None
  if args.hpb_target:hpb=await stack.enter_async_context(LoopbackForward(args.hpb_target,8081 if tls else 8080,8080,ssl_context=tls,listen=args.hpb_target if tls else '127.0.0.1'))
  pw=await stack.enter_async_context(async_playwright())
  browser=await pw.chromium.launch(headless=True,channel='chromium',args=['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--disable-features=WebRtcHideLocalIpsWithMdns','--autoplay-policy=no-user-gesture-required','--webrtc-ip-handling-policy=default_public_and_private_interfaces','--force-webrtc-ip-handling-policy=default_public_and_private_interfaces','--auto-select-desktop-capture-source=Entire screen','--enable-usermedia-screen-capturing','--allow-http-screen-capture'])
  context=await browser.new_context(viewport={'width':1280,'height':900},locale='en-US',permissions=['camera','microphone']);await context.add_init_script('('+INSTRUMENT+')()');page=await context.new_page();room=(FreshDirect if args.scenario.startswith('incoming-') else FreshRoom)(m['client_url'],c['users']['lab_outsider' if args.scenario.startswith('incoming-') else 'lab_alice'],args.manifest.parent/'normal-owned-rooms');process=None;native_errors=bytearray();error_task=None;extra_context=None;extra_page=None
  caller='lab_outsider' if args.scenario.startswith('incoming-') else 'lab_bob'
  with tempfile.TemporaryDirectory(prefix='nmt-normal-call-',dir=os.environ.get('XDG_RUNTIME_DIR','/run')) as directory:
   private=Path(directory);fixture=private/'credentials.json';output=private/'result.json'
   try:
    report['stage']='create room';token=await room.open()
    # A fresh direct room initially contains only its caller. The callee cannot
    # read room-scoped settings until the browser's first call/message admits it.
    report['stage']='caller signaling settings'
    report['signaling_mode']=(await asyncio.to_thread(fetch_settings,m['client_url'],c['users'][caller],token,caller)).get('signalingMode','unknown');fixture.write_text(json.dumps({'origin':m['client_url'],'user':'lab_alice','password':c['users']['lab_alice'],'room':{'token':token}}));fixture.chmod(0o600)
    recovery_window=50 if report['signaling_mode']=='internal' and args.scenario in ('network-resume','web-screen-resume') else 32
    report['exchange_recovery_window_seconds']=recovery_window
    report['stage']='browser login';await page.goto(m['client_url']+'/login',wait_until='domcontentloaded',timeout=30000);await page.locator('#user').fill(caller);await page.locator('#password').fill(c['users'][caller]);await page.get_by_role('button',name='Log in',exact=True).click();await page.wait_for_url(lambda url:'/login' not in urlparse(url).path,timeout=30000)
    if args.participants==3:
     await asyncio.to_thread(room.request,'POST','/'+token+'/participants',{'newParticipant':'lab_outsider','source':'users'})
     extra_context=await browser.new_context(viewport={'width':1280,'height':900},locale='en-US',permissions=['camera','microphone']);await extra_context.add_init_script('('+INSTRUMENT+')()');extra_page=await extra_context.new_page()
     await extra_page.goto(m['client_url']+'/login',wait_until='domcontentloaded',timeout=30000);await extra_page.locator('#user').fill('lab_outsider');await extra_page.locator('#password').fill(c['users']['lab_outsider']);await extra_page.get_by_role('button',name='Log in',exact=True).click();await extra_page.wait_for_url(lambda url:'/login' not in urlparse(url).path,timeout=30000)
     await extra_page.goto(m['client_url']+'/index.php/call/'+token,wait_until='domcontentloaded',timeout=30000);await extra_page.wait_for_timeout(1800);await press(extra_page,['Close','Dismiss','Skip'])
     for _ in range(5):
      if await extra_page.get_by_role('button',name='Leave call',exact=True).count():break
      await press(extra_page,['Start call','Join call','Join the call','Join']);await extra_page.wait_for_timeout(1000)
    if args.order=='native-first':process,native_errors,error_task=await spawn_native(args.executable,fixture,output,args.scenario,recovery_window)
    report['stage']='browser call';await page.goto(m['client_url']+'/index.php/call/'+token,wait_until='domcontentloaded',timeout=30000);await page.wait_for_timeout(1800);await press(page,['Close','Dismiss','Skip']);report['ui_actions']=[]
    for _ in range(5):
     if await page.get_by_role('button',name='Leave call',exact=True).count():break
     action=await press(page,['Start call','Join call','Join the call','Join'])
     if action:report['ui_actions'].append(action)
     await page.wait_for_timeout(1000)
    report['browser_pre_call']=await page.evaluate("({buttons:[...document.querySelectorAll('button')].slice(0,32).map(b=>(b.getAttribute('aria-label')||b.innerText||'').slice(0,80)),text:document.body.innerText.slice(0,1800)})")
    report['stage']='normal native exchange'
    if process is None:process,native_errors,error_task=await spawn_native(args.executable,fixture,output,args.scenario,recovery_window)
    samples=[];start=time.monotonic();policy_changed=False;screen_started=False;screen_stopped=False;network_cut=False;peer_rejoined=False;fault_sample=None;extra_samples=[]
    while process.returncode is None and time.monotonic()-start<100:
     progress={}
     try:
      if output.exists():progress=json.loads(output.read_text())
     except (OSError,ValueError):pass
     ready=progress.get('media_ready_ms');timeline=progress.get('samples',[]);elapsed=(timeline[-1]['ms']-ready)/1000 if ready is not None and timeline else None
     if len(samples)<192:samples.append(await page.evaluate('window.__nmt_stats()'))
     if extra_page and len(extra_samples)<192:extra_samples.append(await extra_page.evaluate('window.__nmt_stats()'))
     if args.scenario in ('network-resume','session-replaced','web-screen-resume') and not network_cut and elapsed is not None and elapsed>=9 and all(media_checks(samples).values()) and (not extra_page or all(media_checks(extra_samples).values())):
      report['fault_after_initial_media']=True;fault_sample=len(samples)-1;report['fault_native_ms']=timeline[-1]['ms']
      if args.scenario=='session-replaced':
       if not args.lab_state:raise RuntimeError('owned lab state required')
       restart=await asyncio.create_subprocess_exec('/usr/bin/python3',str(Path(__file__).with_name('hpb_lab.py')),'--state',str(args.lab_state),'--restart-signaling',stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.DEVNULL)
       if await asyncio.wait_for(restart.wait(),15)!=0:raise RuntimeError('signaling restart failed')
       report['signaling_process_replaced']=True
      else:await (hpb or main_forward).interrupt(1.5);report['control_outage_ms']=1500
      network_cut=True
     if args.rejoin_web_peer_after_replacement and network_cut and not peer_rejoined and progress.get('automatic_new_call_generation'):
      await page.wait_for_timeout(800);leave=await press(page,['Leave call']);await page.wait_for_timeout(1000);join=None
      for _ in range(5):
       if await page.get_by_role('button',name='Leave call',exact=True).count():break
       join=await press(page,['Start call','Join call','Join the call','Join']);await page.wait_for_timeout(1000)
      peer_rejoined=True;report['explicit_browser_rejoin_fixture']={'leave':leave,'join':join,'joined':bool(await page.get_by_role('button',name='Leave call',exact=True).count())}
     if args.refresh_web_peer_flags_after_replacement and network_cut and not peer_rejoined and progress.get('automatic_new_call_generation'):
      # Diagnostic intervention in the owned peer, never automatic native
      # recovery. Reassert only the synthetic web participant's own AV flags.
      status=await page.evaluate("""async url => (await fetch(url,{method:'PUT',headers:{'OCS-APIRequest':'true','Accept':'application/json','requesttoken':OC.requestToken,'Content-Type':'application/x-www-form-urlencoded'},body:'flags=7'})).status""",m['client_url']+'/ocs/v2.php/apps/spreed/api/v4/call/'+token)
      peer_rejoined=True;report['explicit_browser_flags_refresh_fixture']={'status':status,'no_leave_or_rejoin':True}
     if args.scenario in ('web-screen','web-screen-resume') and not screen_started and (elapsed is not None and elapsed>=5 if args.scenario=='web-screen-resume' else time.monotonic()-start>=5):
      report['screen_start_action']=await press(page,['Enable screensharing','Share screen','Start screen sharing','Share screen (S)']);screen_started=True
      report['screen_buttons']=[x[:80] for x in await page.locator('button').all_text_contents()][:40]
     if args.scenario in ('web-screen','web-screen-resume') and screen_started and not screen_stopped and (elapsed is not None and elapsed>=(44 if recovery_window==50 else 16) if args.scenario=='web-screen-resume' else time.monotonic()-start>=16):
      report['screen_stop_action']=await press(page,['Disable screensharing','Stop screensharing','Stop screen sharing','Stop sharing','Stop sharing screen']);screen_stopped=True
     if args.scenario=='policy-loss' and not policy_changed and time.monotonic()-start>=11:
      await asyncio.to_thread(room.request,'PUT','/'+token+'/read-only',{'state':1});policy_changed=True;report['policy_change_ms']=int((time.monotonic()-start)*1000)
     try:await asyncio.wait_for(process.wait(),.5)
     except TimeoutError:pass
    if process.returncode is None:process.kill();await process.wait();report['deadline_exceeded']=True
    report['exit']=process.returncode;report['native_log']=native_errors.decode(errors='replace');report['native']=json.loads(output.read_text()) if output.exists() else {};report['browser_samples']=samples;checks=media_checks(samples);channels=[c for sample in samples for peer in sample for c in peer.get('data_channels',[])];checks['sctp_sent']=any(c.get('sent',0)>=2 for c in channels);checks['sctp_received']=any(c.get('received',0)>=2 for c in channels);
    if args.scenario in ('native-screen','native-static-screen','portal-screen'):checks['screen_received_at_720p']=any(r.get('type')=='inbound-rtp' and r.get('frameWidth')==1280 and r.get('frameHeight')==720 and r.get('framesDecoded',0)>10 for sample in samples for peer in sample for r in peer.get('rows',[]))
    if args.scenario in ('network-resume','session-replaced','web-screen-resume'):
     timeline=report['native'].get('samples',[]);recovery=native_recovery(timeline,report.get('fault_native_ms',float('inf')));checks.update({k:recovery[k] for k in ('reconnecting_observed','media_resumed')});checks['fault_after_initial_media']=report.get('fault_after_initial_media',False)
     report['recovery_assessment_sha256']=digest(Path(__file__).with_name('recovery_checks.py'))
     def resumed_browser(rows):
      def grew(kind,field,minimum):
       before=rows[fault_sample] if fault_sample is not None and fault_sample<len(rows) else []
       baseline={i:max((r.get(field,0) for r in peer.get('rows',[]) if r.get('type')=='inbound-rtp' and r.get('kind')==kind),default=0) for i,peer in enumerate(before)}
       return fault_sample is not None and any(r.get(field,0)>baseline.get(i,0)+minimum for sample in rows[fault_sample+1:] for i,peer in enumerate(sample) for r in peer.get('rows',[]) if r.get('type')=='inbound-rtp' and r.get('kind')==kind)
      return grew('audio','packetsReceived',100) and grew('video','framesDecoded',10)
     checks['browser_media_resumed']=resumed_browser(samples)
     if extra_page:checks['third_party_media_resumed']=resumed_browser(extra_samples)
     if args.scenario=='web-screen-resume':checks['screen_flow_after_recovery']=recovery['screen_flow_after_recovery']
    if args.rejoin_web_peer_after_replacement:checks['explicit_browser_rejoin_complete']=report.get('explicit_browser_rejoin_fixture',{}).get('joined',False)
    if args.refresh_web_peer_flags_after_replacement:checks['explicit_browser_flags_refresh_complete']=report.get('explicit_browser_flags_refresh_fixture',{}).get('status')==200
    if extra_page:checks['third_party_av']=all(media_checks(extra_samples).values());report['third_party_samples']=extra_samples
    report['browser_checks']=checks;report['result']='pass' if (all(checks.values()) or args.scenario=='incoming-decline') and report['native'].get('result')=='pass' else 'fail'
    if policy_changed:await asyncio.to_thread(room.request,'PUT','/'+token+'/read-only',{'state':0})
   except Exception as error:
    report['failure_operation']=str(error).split('\n')[0][:160] if isinstance(error,TimeoutError) else type(error).__name__
    try:report['socket_states']=await page.evaluate('window.__nmt_sockets');report['browser_failure_view']=await page.evaluate("({buttons:[...document.querySelectorAll('button')].slice(0,40).map(b=>(b.getAttribute('aria-label')||b.innerText||'').slice(0,80)),text:document.body.innerText.slice(0,1600)})")
    except Exception:pass
    raise
   finally:
    if process and process.returncode is None:process.kill();await process.wait()
    if error_task:
     try:await asyncio.wait_for(error_task,2)
     except TimeoutError:error_task.cancel()
    try:report['browser_leave']=await press(page,['Leave call','End call'])
    except Exception:report['browser_leave']='unavailable'
    if extra_context:await extra_context.close()
    await context.close();await browser.close();report['room_cleanup']=await room.close();report['private_fixture_removed']=True
    if report['room_cleanup']!='absence_confirmed':report['result']='fail'
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--manifest',type=Path,required=True);p.add_argument('--executable',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--scenario',choices=['media','reconnect','policy-loss','incoming-accept','incoming-decline','native-screen','native-static-screen','portal-screen','web-screen','network-resume','session-replaced','web-screen-resume'],default='media');p.add_argument('--participants',type=int,choices=[2,3],default=2);p.add_argument('--hpb-target');p.add_argument('--lab-state',type=Path);p.add_argument('--rejoin-web-peer-after-replacement',action='store_true',help='Qualification only: explicitly leave/rejoin the web peer after HPB replacement');p.add_argument('--refresh-web-peer-flags-after-replacement',action='store_true',help='Diagnostic only: reassert the owned web peer call flags after native fresh admission');p.add_argument('--order',choices=['browser-first','native-first'],default='browser-first');args=p.parse_args()
 if (args.rejoin_web_peer_after_replacement or args.refresh_web_peer_flags_after_replacement) and (args.scenario!='session-replaced' or not args.hpb_target):p.error('web peer intervention requires session-replaced with an owned HPB target')
 if args.rejoin_web_peer_after_replacement and args.refresh_web_peer_flags_after_replacement:p.error('choose only one web peer intervention')
 report={'workflow':'normal-talk-browser','result':'fail','harness_sha256':digest(__file__),'physical_devices':False,'scenario':args.scenario,'order':args.order,'automatic_recovery_fixture':not (args.rejoin_web_peer_after_replacement or args.refresh_web_peer_flags_after_replacement)}
 try:asyncio.run(run(args,report))
 except Exception as error:
  report['failure_type']=type(error).__name__
  report['failure_flags']={key:key in str(error) for key in ['Permission denied','Executable doesn\'t exist','Failed to launch','Sandbox','not found']}
  if isinstance(error,OSError):report['failure_errno']=error.errno
 args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'result':report['result'],'stage':report.get('stage'),'output':str(args.output)}));raise SystemExit(0 if report['result']=='pass' else 1)
if __name__=='__main__':main()
