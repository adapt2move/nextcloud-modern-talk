#!/usr/bin/env python3
"""Finite original Talk browser against the normal production account call path."""
import argparse,asyncio,base64,json,tempfile,time,urllib.request
from pathlib import Path
from urllib.parse import urlparse,urlencode
from playwright.async_api import async_playwright
from talk_browser import LoopbackForward,FreshRoom,INSTRUMENT,press,media_checks,private_json,digest
# Observe original browser SCTP messages without retaining any payloads.
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
async def run(args,report):
 m=private_json(args.manifest);c=private_json(m['credentials_file']);origin=urlparse(m['client_url']);target=urlparse(m['vm_url'])
 if not m['ready'] or origin.hostname!='127.0.0.1' or origin.scheme!='http':raise RuntimeError('fixture origin')
 report.update(profile=m['profile'],nextcloud=m['nextcloud'],talk=m['talk'],executable_sha256=digest(args.executable),instrument_dependency_sha256=digest(Path(__file__).with_name('talk_browser.py')),room_cleanup_scope='synthetic initiator membership; remaining direct pair participant removed by lab teardown')
 async with LoopbackForward(target.hostname,origin.port),async_playwright() as pw:
  browser=await pw.chromium.launch(headless=True,channel='chromium',args=['--use-fake-device-for-media-stream','--use-fake-ui-for-media-stream','--disable-features=WebRtcHideLocalIpsWithMdns','--autoplay-policy=no-user-gesture-required','--webrtc-ip-handling-policy=default_public_and_private_interfaces','--force-webrtc-ip-handling-policy=default_public_and_private_interfaces'])
  context=await browser.new_context(viewport={'width':1280,'height':900},locale='en-US',permissions=['camera','microphone']);await context.add_init_script('('+INSTRUMENT+')()');page=await context.new_page();room=(FreshDirect if args.scenario.startswith('incoming-') else FreshRoom)(m['client_url'],c['users']['lab_outsider' if args.scenario.startswith('incoming-') else 'lab_alice'],args.manifest.parent/'normal-owned-rooms');process=None
  caller='lab_outsider' if args.scenario.startswith('incoming-') else 'lab_bob'
  with tempfile.TemporaryDirectory(prefix='nmt-normal-call-',dir='/run') as directory:
   private=Path(directory);fixture=private/'credentials.json';output=private/'result.json'
   try:
    report['stage']='create room';token=await room.open();fixture.write_text(json.dumps({'origin':m['client_url'],'user':'lab_alice','password':c['users']['lab_alice'],'room':{'token':token}}));fixture.chmod(0o600)
    report['stage']='browser login';await page.goto(m['client_url']+'/login',wait_until='domcontentloaded',timeout=30000);await page.locator('#user').fill(caller);await page.locator('#password').fill(c['users'][caller]);await page.get_by_role('button',name='Log in',exact=True).click();await page.wait_for_url(lambda url:'/login' not in urlparse(url).path,timeout=30000)
    if args.order=='native-first':process=await asyncio.create_subprocess_exec(str(args.executable),str(fixture),str(output),args.scenario,stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.PIPE)
    report['stage']='browser call';await page.goto(m['client_url']+'/index.php/call/'+token,wait_until='domcontentloaded',timeout=30000);await page.wait_for_timeout(1800);await press(page,['Close','Dismiss','Skip']);report['ui_actions']=[]
    for _ in range(5):
     if await page.get_by_role('button',name='Leave call',exact=True).count():break
     action=await press(page,['Start call','Join call','Join the call','Join'])
     if action:report['ui_actions'].append(action)
     await page.wait_for_timeout(1000)
    report['browser_pre_call']=await page.evaluate("({buttons:[...document.querySelectorAll('button')].slice(0,32).map(b=>(b.getAttribute('aria-label')||b.innerText||'').slice(0,80)),text:document.body.innerText.slice(0,1800)})")
    report['stage']='normal native exchange'
    if process is None:process=await asyncio.create_subprocess_exec(str(args.executable),str(fixture),str(output),args.scenario,stdout=asyncio.subprocess.DEVNULL,stderr=asyncio.subprocess.PIPE)
    samples=[];start=time.monotonic();policy_changed=False
    while process.returncode is None and time.monotonic()-start<65:
     if len(samples)<120:samples.append(await page.evaluate('window.__nmt_stats()'))
     if args.scenario=='policy-loss' and not policy_changed and time.monotonic()-start>=11:
      await asyncio.to_thread(room.request,'PUT','/'+token+'/read-only',{'state':1});policy_changed=True;report['policy_change_ms']=int((time.monotonic()-start)*1000)
     try:await asyncio.wait_for(process.wait(),.5)
     except TimeoutError:pass
    if process.returncode is None:process.kill();await process.wait();report['deadline_exceeded']=True
    report['exit']=process.returncode;report['native_log']=(await process.stderr.read(4097)).decode(errors='replace')[:4096];report['native']=json.loads(output.read_text()) if output.exists() else {};report['browser_samples']=samples;checks=media_checks(samples);channels=[c for sample in samples for peer in sample for c in peer.get('data_channels',[])];checks['sctp_sent']=any(c.get('sent',0)>=2 for c in channels);checks['sctp_received']=any(c.get('received',0)>=2 for c in channels);report['browser_checks']=checks;report['result']='pass' if (all(checks.values()) or args.scenario=='incoming-decline') and report['native'].get('result')=='pass' else 'fail'
    if policy_changed:await asyncio.to_thread(room.request,'PUT','/'+token+'/read-only',{'state':0})
   finally:
    if process and process.returncode is None:process.kill();await process.wait()
    try:report['browser_leave']=await press(page,['Leave call','End call'])
    except Exception:report['browser_leave']='unavailable'
    await context.close();await browser.close();report['room_cleanup']=await room.close();report['private_fixture_removed']=True
    if report['room_cleanup']!='absence_confirmed':report['result']='fail'
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--manifest',type=Path,required=True);p.add_argument('--executable',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--scenario',choices=['media','reconnect','policy-loss','incoming-accept','incoming-decline'],default='media');p.add_argument('--order',choices=['browser-first','native-first'],default='browser-first');args=p.parse_args();report={'workflow':'normal-talk-browser','result':'fail','harness_sha256':digest(__file__),'physical_devices':False,'scenario':args.scenario,'order':args.order}
 try:asyncio.run(run(args,report))
 except Exception as error:report['failure_type']=type(error).__name__
 args.output.write_text(json.dumps(report,indent=2)+'\n');print(json.dumps({'result':report['result'],'stage':report.get('stage'),'output':str(args.output)}));raise SystemExit(0 if report['result']=='pass' else 1)
if __name__=='__main__':main()
