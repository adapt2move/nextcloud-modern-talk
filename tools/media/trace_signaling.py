#!/usr/bin/env python3
"""Bounded HPB wire metadata observer around the original Talk interoperability driver.

Pass the normal_talk.py arguments. Forwarded bytes are unchanged; credentials,
SDP and candidates are never recorded. Identities are represented by hashes.
"""
import asyncio,contextvars,hashlib,json,sys,time
from pathlib import Path
sys.path.insert(0,str(Path.cwd()/'tools/media'))
import normal_talk
from signaling_metadata import Requests
from talk_browser import LoopbackForward
script_sha256=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
http_observer_sha256=hashlib.sha256(Path(__file__).with_name('signaling_metadata.py').read_bytes()).hexdigest()
started=time.monotonic();trace=[];connection=contextvars.ContextVar('connection');counter=0;roles={}
def tag(v):
 return hashlib.sha256(v.encode()).hexdigest()[:10] if isinstance(v,str) and v else None
def observe(c,d,v):
 if len(trace)>=1600:return
 row={'ms':round((time.monotonic()-started)*1000),'connection':c,'role':roles.get(c),'direction':d,'type':v.get('type'),'id':v.get('id')}
 kind=v.get('type')
 if kind=='hello':
  h=v.get('hello',{});row.update(session=tag(h.get('sessionid')),resume='resumeid' in h and d=='out')
 elif kind=='room':row['room_session']=tag(v.get('room',{}).get('sessionid'))
 elif kind=='event':
  e=v.get('event',{});row.update(target=e.get('target'),event=e.get('type'))
  if e.get('type')=='join':row['members']=[{'sfu':tag(x.get('sessionid')),'talk':tag(x.get('roomsessionid'))} for x in e.get('join',[])][:100]
  if e.get('type')=='leave':row['left']=[tag(x) for x in e.get('leave',[])][:100]
  if e.get('target')=='participants':
   u=e.get('update',{});row.update(all=u.get('all'),incall=u.get('incall'),users=[{'sfu':tag(x.get('sessionId')),'talk':tag(x.get('nextcloudSessionId')),'incall':x.get('inCall')} for x in u.get('users',[])][:100])
 elif kind=='message':
  m=v.get('message',{});data=m.get('data',{});row.update(signal=data.get('type'),stream=data.get('roomType'),sender=tag(m.get('sender',{}).get('sessionid')),recipient=tag(m.get('recipient',{}).get('sessionid')),sid=tag(data.get('sid')))
 elif kind=='error':row['code']=v.get('error',{}).get('code')
 elif kind not in ('bye','welcome'):return
 if row.get('signal') in ('candidate','endOfCandidates'):return
 trace.append(row)
class Frames:
 def __init__(self,c,d):self.c,self.d,self.buf,self.headers=c,d,bytearray(),False
 def feed(self,data):
  self.buf.extend(data)
  if not self.headers:
   end=self.buf.find(b'\r\n\r\n')
   if end<0:
    if len(self.buf)>16384:self.buf.clear()
    return
   header=bytes(self.buf[:end]);del self.buf[:end+4];self.headers=True
   if self.d=='out':roles[self.c]='browser' if b'Chrome/' in header else 'native'
  if len(self.buf)>262144:self.buf.clear();return
  while len(self.buf)>=2:
   first,second=self.buf[:2];length=second&127;offset=2
   if length==126:
    if len(self.buf)<4:return
    length=int.from_bytes(self.buf[2:4],'big');offset=4
   elif length==127:
    if len(self.buf)<10:return
    length=int.from_bytes(self.buf[2:10],'big');offset=10
   if length>131072:self.buf.clear();return
   masked=bool(second&128)
   key=bytes(self.buf[offset:offset+4]) if masked else None
   if masked:offset+=4
   if len(self.buf)<offset+length:return
   payload=bytes(self.buf[offset:offset+length]);del self.buf[:offset+length]
   if first&15!=1 or not first&128:continue
   if masked:payload=bytes(b^key[i%4] for i,b in enumerate(payload))
   try:observe(self.c,self.d,json.loads(payload))
   except (ValueError,TypeError,AttributeError):pass
class Inspect(LoopbackForward):
 async def accept(self,reader,writer):
  global counter
  counter+=1;token=connection.set(counter)
  try:await super().accept(reader,writer)
  finally:connection.reset(token)
 async def pipe(self,reader,writer):
  if self.target_port!=8080:
   if writer.get_extra_info('peername')[1]!=self.target_port:return await super().pipe(reader,writer)
   requests=Requests(connection.get(),trace,started)
   while True:
    data=await asyncio.wait_for(reader.read(65536),45)
    if not data:return
    requests.feed(data);writer.write(data);await asyncio.wait_for(writer.drain(),10)
  direction='out' if writer.get_extra_info('peername')[1]==8080 else 'in'
  frames=Frames(connection.get(),direction)
  while True:
   data=await asyncio.wait_for(reader.read(65536),45)
   if not data:return
   frames.feed(data);writer.write(data);await asyncio.wait_for(writer.drain(),10)
normal_talk.LoopbackForward=Inspect
output=Path(sys.argv[sys.argv.index('--output')+1])
try:normal_talk.main()
finally:output.with_name(output.stem+'-wire.json').write_text(json.dumps({'script_sha256':script_sha256,'http_observer_sha256':http_observer_sha256,'trace':trace},indent=2)+'\n')
