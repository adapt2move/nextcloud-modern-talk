#!/usr/bin/env python3
"""Focused native settings exercise. Real computer writer; isolated test profile.

Conversations remain explicit disconnected development fixtures. OS loopback
input, when enabled, is labelled SYNTHETIC DEVICE and never physical media proof.
"""
import argparse,csv,hashlib,io,json,os,pwd,re,select,signal,subprocess,time,unicodedata
from pathlib import Path

def sha(p):return hashlib.sha256(Path(p).read_bytes()).hexdigest()
class Native:
 def __init__(self,a):
  self.a=a;self.out=a.output.resolve();self.out.mkdir(parents=True,exist_ok=False)
  self.env=os.environ.copy();self.env.pop('WAYLAND_DISPLAY',None)
  # agent-dev-box exec has a minimal environment; restore the real account's
  # standard home (not a test-directory override). Preferences use isolated XDG.
  self.env.setdefault('HOME',pwd.getpwuid(os.getuid()).pw_dir)
  self.env['VK_ICD_FILENAMES']=str(next(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json')))
  self.env['XDG_RUNTIME_DIR']=str(self.out/'runtime');Path(self.env['XDG_RUNTIME_DIR']).mkdir(mode=0o700)
  self.env['XDG_CONFIG_HOME']=str(self.out/'profile');Path(self.env['XDG_CONFIG_HOME']).mkdir()
  self.jobs=[];self.apps=[];self.events=[];self.frames=[];self.assertions=[];self.logs=[];self.pulse_module=None;self.camera_module_owned=False;self.camera_producer=None
 def run(self,*args,timeout=20):return subprocess.run(list(args),env=self.env,text=True,capture_output=True,check=True,timeout=timeout).stdout
 def spawn(self,name,args,**kw):
  f=(self.out/(name+'.log')).open('w');self.logs.append(f)
  p=subprocess.Popen(args,env=self.env,stdout=f,stderr=f,start_new_session=True,**kw);self.jobs.append(p);return p
 def start(self):
  r,w=os.pipe();self.spawn('xvfb',['Xvfb','-displayfd',str(w),'-screen','0','1600x1200x24','-nolisten','tcp','-noreset'],pass_fds=(w,));os.close(w)
  assert select.select([r],[],[],15)[0];self.env['DISPLAY']=':'+os.read(r,32).decode().strip();os.close(r)
  bus=subprocess.Popen(['dbus-daemon','--session','--nofork','--print-address=1'],stdout=subprocess.PIPE,stderr=(self.out/'dbus.log').open('w'),env=self.env,start_new_session=True);self.jobs.append(bus)
  assert select.select([bus.stdout],[],[],10)[0];self.env['DBUS_SESSION_BUS_ADDRESS']=bus.stdout.readline().decode().strip()
  self.spawn('window-manager',['openbox']);self.spawn('notifications',['dunst','-config','/dev/null']);time.sleep(.6)
 def synthetic_audio(self):
  socket=str(self.out/'runtime/pulse-native');self.env['PULSE_SERVER']='unix:'+socket
  cookie=self.out/'runtime/pulse-cookie';cookie.write_bytes(os.urandom(256));cookie.chmod(0o600);self.env['PULSE_COOKIE']=str(cookie)
  self.spawn('pulse',['pulseaudio','--daemonize=no','--exit-idle-time=-1','--use-pid-file=no','-n','-L','module-native-protocol-unix socket='+socket+' auth-cookie='+str(cookie)])
  deadline=time.monotonic()+10
  while not Path(socket).exists() and time.monotonic()<deadline:time.sleep(.1)
  self.pulse_module=self.run('pactl','load-module','module-null-sink','sink_name=nmt_device_test','sink_properties=device.description=NMT_Synthetic_DEVICE').strip()
  self.run('pactl','set-default-sink','nmt_device_test');self.run('pactl','set-default-source','nmt_device_test.monitor')
 def synthetic_devices(self):
  self.synthetic_audio()
  if not Path('/sys/module/v4l2loopback').exists():
   self.run('modprobe','v4l2loopback','video_nr=42','card_label=NMT Synthetic DEVICE Camera','exclusive_caps=1');self.camera_module_owned=True
  else:raise RuntimeError('Existing loopback module is outside this recipe ownership')
  self.camera_producer=self.spawn('synthetic-camera',['gst-launch-1.0','-q','videotestsrc','is-live=true','pattern=ball','!','video/x-raw,width=640,height=480,framerate=30/1,format=YUY2','!','v4l2sink','device=/dev/video42'])
  self.events.append({'synthetic_os_devices':True,'pulse_module':self.pulse_module,'v4l2':'/dev/video42','camera_module_owned':self.camera_module_owned});time.sleep(1)
 def app(self,name,scenario='settings',preserve=False,extra=(),activate=True):
  args=[str(self.a.binary.resolve())]
  if scenario:args+=['--ui-scenario',scenario]
  if preserve:args+=['--ui-preserve-preferences']
  elif scenario:args+=['--ui-appearance','light','--ui-theme','blue','--ui-locale','de']
  args+=list(extra);self.app_log=self.out/(name+'.log');self.current=self.spawn(name,args);self.apps.append(self.current)
  deadline=time.monotonic()+25;self.xid=None
  while time.monotonic()<deadline:
   if self.current.poll() is not None:raise RuntimeError('App exited: '+self.app_log.read_text()[-3000:])
   p=subprocess.run(['xdotool','search','--pid',str(self.current.pid),'--class','talk-desktop'],env=self.env,text=True,capture_output=True)
   if p.returncode==0 and p.stdout.strip():self.xid=p.stdout.splitlines()[-1];break
   p=subprocess.run(['xdotool','search','--onlyvisible','--pid',str(self.current.pid)],env=self.env,text=True,capture_output=True)
   if p.returncode==0 and p.stdout.strip():self.xid=p.stdout.splitlines()[0];break
   time.sleep(.1)
  if not self.xid:raise RuntimeError('No native window')
  self.run('xdotool','windowsize','--sync',self.xid,str(self.a.width),str(self.a.height))
  if activate:self.run('xdotool','windowactivate','--sync',self.xid)
  time.sleep(.6)
  self.events.append({'start':args,'pid':self.current.pid})
 def key(self,key):
  self.run('xdotool','key','--clearmodifiers',key);self.events.append({'key':key});time.sleep(.18)
 def click(self,x,y):
  self.run('xdotool','mousemove','--window',self.xid,str(x),str(y));self.run('xdotool','click','1');self.events.append({'click':[x,y]});time.sleep(.25)
 def type(self,text):self.run('xdotool','type','--clearmodifiers','--delay','35',text);self.events.append({'typed_fixture':text});time.sleep(.2)
 def prefs(self):
  path=Path(self.env['XDG_CONFIG_HOME'])/'nextcloud-modern-talk/preferences.conf'
  return dict(line.split('=',1) for line in path.read_text().splitlines()[1:])
 def saved(self,**expected):
  deadline=time.monotonic()+8
  while time.monotonic()<deadline:
   try:
    actual=self.prefs()
    if all(actual.get(k)==v for k,v in expected.items()):break
   except (FileNotFoundError,IsADirectoryError):actual={}
   time.sleep(.1)
  self.expect('durable-values-match',all(actual.get(k)==v for k,v in expected.items()),{'expected':expected,'actual':actual})
  return actual
 def scroll(self,steps):
  self.run('xdotool','mousemove','--window',self.xid,'930','550')
  self.run('xdotool','click','--repeat',str(abs(steps)),'--delay','90','5' if steps>0 else '4');time.sleep(.35)
 def click_text(self,phrase,dy=0):
  # Read the actual native pixels, not a fixture element tree. Preserve raw OCR.
  scratch=self.out/'scratch';scratch.mkdir(exist_ok=True)
  path=scratch/(str(len(self.events))+'.png');self.run('import','-window',self.xid,str(path))
  raw=self.run('tesseract',str(path),'stdout','-l','deu+eng','--psm','11','tsv');path.with_suffix('.tsv').write_text(raw)
  # OCR sometimes includes a quote from the adjacent selection marker. Keep
  # whole-label matching; remove only framing punctuation, never label words.
  def norm(s):return ''.join(c for c in unicodedata.normalize('NFD',s.lower()) if not unicodedata.combining(c)).strip().strip("‘’'\"|[]")
  groups={}
  for row in csv.DictReader(io.StringIO(raw),delimiter='\t',quoting=csv.QUOTE_NONE):
   if row['level']=='5' and row['text'].strip():groups.setdefault((row['block_num'],row['par_num'],row['line_num']),[]).append(row)
  for rows in groups.values():
   words=[norm(row['text']) for row in rows];target=norm(phrase).split()
   for i in range(len(words)-len(target)+1):
    if words[i:i+len(target)]==target:
     span=rows[i:i+len(target)];left=min(int(r['left']) for r in span);top=min(int(r['top']) for r in span)
     right=max(int(r['left'])+int(r['width']) for r in span);bottom=max(int(r['top'])+int(r['height']) for r in span)
     self.events.append({'native_ocr_target':phrase,'bounds':[left,top,right,bottom],'control_offset_y':dy});self.click((left+right)//2,(top+bottom)//2+dy);return
  raise AssertionError('Full native label not found: '+phrase)
 def diag(self):
  self.key('F7');lines=self.app_log.read_text().splitlines()
  return {k:next((x for x in reversed(lines) if x.startswith(k)),'') for k in ['UI_SETTINGS','UI_DEVICES','UI_NOTIFICATIONS','UI_LOCAL_STATE','UI_ACTION_FOCUS','UI_HISTORY','UI_GEOMETRY']}
 def frame(self,name,root=False,delay=.35):
  time.sleep(delay);path=self.out/(name+'.png');self.run('import','-window','root' if root else self.xid,str(path))
  ocr=self.run('tesseract',str(path),'stdout','-l','deu+eng','--psm','11');(self.out/(name+'-raw-ocr.txt')).write_text(ocr)
  item={'name':name,'path':path.name,'sha256':sha(path),'events':self.events.copy(),'diagnostics':self.diag(),'ocr':ocr};self.frames.append(item);return item
 def expect(self,name,value,evidence):
  self.assertions.append({'name':name,'pass':bool(value),'evidence':evidence})
  if not value:raise AssertionError(name+': '+str(evidence))
 def tab(self,index):
  # Fixed navigation rail in the declared 1120x716 native size. The diagnostic
  # confirms the actual selected tab; names alone never stand in for activation.
  self.click(238,127+41*index)
 def close_app(self):
  self.run('xdotool','key','--clearmodifiers','alt+F4');deadline=time.monotonic()+15
  while self.current.poll() is None and time.monotonic()<deadline:time.sleep(.05)
  self.expect('process-exits-after-owned-worker-drain',self.current.poll()==0,{'exit':self.current.poll()})
 def finish(self,error=None):
  cleanup=[]
  # Ask the actual app to drain before terminating the owned native fixtures.
  for p in self.apps:
   if p.poll() is None:
    try:self.run('xdotool','windowactivate','--sync',self.xid);self.run('xdotool','key','alt+F4');p.wait(timeout=12)
    except (subprocess.SubprocessError,ProcessLookupError):pass
  if self.camera_producer and self.camera_producer.poll() is None:
   os.killpg(self.camera_producer.pid,signal.SIGTERM);self.camera_producer.wait(timeout=5)
  if self.camera_module_owned:
   r=subprocess.run(['modprobe','-r','v4l2loopback'],env=self.env,capture_output=True,text=True);cleanup.append({'remove_owned_camera_module':r.returncode,'stderr':r.stderr})
  if self.pulse_module:
   r=subprocess.run(['pactl','unload-module',self.pulse_module],env=self.env,capture_output=True,text=True);cleanup.append({'unload_owned_pulse_module':r.returncode,'stderr':r.stderr})
  # Only explicitly owned child processes. No daemon/session/global cleanup.
  for p in reversed(self.jobs):
   if p.poll() is None:
    os.killpg(p.pid,signal.SIGTERM)
    try:p.wait(timeout=5)
    except subprocess.TimeoutExpired:os.killpg(p.pid,signal.SIGKILL);p.wait()
  for f in self.logs:f.close()
  mount=self.out/'runtime/doc'
  if os.path.ismount(mount):
   r=subprocess.run(['fusermount3','-u',str(mount)],env=self.env,capture_output=True,text=True);cleanup.append({'owned_portal_mount_removed':r.returncode==0,'stderr':r.stderr})
  cookie=self.out/'runtime/pulse-cookie'
  if cookie.exists():cookie.unlink();cleanup.append({'owned_ephemeral_pulse_cookie_removed':True})
  (self.out/'observations.json').write_text(json.dumps({'binary_sha256':sha(self.a.binary),'recipe_sha256':sha(__file__),'scope':'native author exercise; no account admission; no physical/Mac qualification','frames':self.frames,'assertions':self.assertions,'cleanup':cleanup,'error':error},indent=2,ensure_ascii=False)+'\n')

def preferences(n):
 n.app('preferences',scenario='chat');n.saved(locale='de')
 n.click(470,587);n.type('ALPHA');n.key('ctrl+comma');n.tab(0)
 n.click(541,166);n.saved(locale='en');n.frame('general-english')
 n.click(712,166);n.saved(locale='es');n.frame('general-spanish')
 n.key('Escape');d=n.diag();n.expect('locale-change-keeps-draft-and-focus','editor_chars=5 editor_focused=true' in d['UI_LOCAL_STATE'],d)
 n.type('Z');n.frame('spanish-workspace-draft-retained')
 n.key('ctrl+comma');n.tab(0);n.click(430,166);n.saved(locale='system');f=n.frame('general-system-language')
 n.expect('system-language-english-fallback','General' in f['ocr'] and 'English' in f['ocr'],f['ocr'])
 n.click(712,166);n.saved(locale='es')
 n.tab(1);n.click(858,204);n.saved(appearance='dark');n.click(814,351);n.saved(palette='sage');n.click(838,486);n.saved(sidebar='compact');n.frame('appearance-dark-sage-compact-spanish')
 n.key('Escape');n.frame('active-compact-spanish');n.click(111,175);f=n.frame('archive-authoritative-fixture')
 n.expect('archive-shows-archived-fixture','notizanmich' in re.sub(r'\s+','',f['ocr'].lower()),f['ocr'])
 n.key('ctrl+comma');n.tab(0)
 # Fail a real atomic replacement of only the harness-owned preference file.
 path=Path(n.env['XDG_CONFIG_HOME'])/'nextcloud-modern-talk/preferences.conf';original=path.read_bytes();path.rename(path.with_suffix('.before-failure'));path.mkdir()
 n.click(541,166);time.sleep(.7);f=n.frame('write-failure-restores-confirmed-spanish')
 n.expect('write-failure-restores-confirmed-active','locale=Spanish' in f['diagnostics']['UI_SETTINGS'] and 'save=Failed' in f['diagnostics']['UI_SETTINGS'],f['diagnostics'])
 path.rmdir();path.write_bytes(original);path.chmod(0o600)
 n.click_text('Reintentar');n.saved(locale='en');f=n.frame('explicit-retry-saved-english')
 n.expect('retry-is-durable','save=Idle' in f['diagnostics']['UI_SETTINGS'] and 'locale=English' in f['diagnostics']['UI_SETTINGS'],f['diagnostics'])
 n.click(930,228);n.saved(login='1');n.click(930,305);n.saved(background='1')
 startup=Path(n.env['XDG_CONFIG_HOME'])/'autostart/nextcloud-modern-talk.desktop'
 n.expect('native-autostart-entry',startup.is_file() and '--autostart' in startup.read_text(),startup.read_text() if startup.exists() else 'missing')
 confirmed=n.prefs();n.frame('general-startup-saved');n.close_app()
 n.app('restart',preserve=True);n.expect('separate-process-loads-identical-values',n.prefs()==confirmed,{'before':confirmed,'after':n.prefs()});f=n.frame('restart-saved-appearance')
 n.expect('loaded-projection-applies','load=Loaded' in f['diagnostics']['UI_SETTINGS'] and 'palette=Sage appearance=Dark density=Compact' in f['diagnostics']['UI_SETTINGS'],f['diagnostics']);n.close_app()
 n.app('autostart',preserve=True,extra=['--autostart'],activate=False)
 state=n.run('xprop','-id',n.xid,'WM_STATE');n.expect('saved-background-start-is-iconic','Iconic' in state,state)
 n.run('xdotool','windowactivate','--sync',n.xid);n.close_app()
 # Ordinary launch, no development scenario or preference CLI overrides.
 n.app('ordinary-launch',scenario=None);n.key('ctrl+comma');f=n.frame('ordinary-launch-real-general')
 n.expect('ordinary-launch-is-visible-and-loaded','Language for this app on this computer. Content is not translated.' in f['ocr'] and n.prefs()['locale']=='en',f['ocr']);n.close_app()

def devices(n):
 n.synthetic_devices();n.app('device-settings');n.tab(2)
 f=n.frame('devices-before-explicit-checks')
 n.expect('opening-settings-never-captures','session=None' in f['diagnostics']['UI_DEVICES'] and 'rendered_frame=false' in f['diagnostics']['UI_DEVICES'],f['diagnostics'])
 n.click(600,222);n.frame('microphone-native-inventory');n.click_text('Monitor of NMT_Synthetic_DEVICE');time.sleep(.5)
 n.expect('microphone-selection-is-saved',n.prefs()['microphone'].startswith('id:'),n.prefs()['microphone'])
 n.click_text('Mikrofon testen');time.sleep(.6)
 n.click_text('Systemstandard');n.frame('speaker-native-inventory');n.click_text('NMT_Synthetic_DEVICE');time.sleep(.4)
 n.expect('speaker-selection-is-saved',n.prefs()['speaker'].startswith('id:'),n.prefs()['speaker'])
 n.click_text('Testton abspielen');d=n.diag();match=re.search(r'level=Some\((\d+)\)',d['UI_DEVICES'])
 n.expect('native-tone-reaches-real-input-callback',bool(match) and int(match[1])>100,d)
 n.frame('synthetic-microphone-meter-and-tone',delay=0)
 n.scroll(4);n.frame('camera-controls');n.click_text('Systemstandard');n.frame('camera-native-inventory');n.click_text('NMT Synthetic DEVICE Camera');time.sleep(.4)
 n.expect('camera-selection-is-saved',n.prefs()['camera'].startswith('id:'),n.prefs()['camera'])
 n.click_text('Kameravorschau starten');time.sleep(1);f=n.frame('synthetic-os-camera-live-preview')
 n.expect('camera-frame-is-rendered','camera=Live' in f['diagnostics']['UI_DEVICES'] and 'rendered_frame=true' in f['diagnostics']['UI_DEVICES'],f['diagnostics'])
 n.tab(0);time.sleep(.5);f=n.frame('leaving-devices-stops-preview')
 n.expect('leaving-tab-revokes-all-capture','session=None' in f['diagnostics']['UI_DEVICES'] and 'rendered_frame=false' in f['diagnostics']['UI_DEVICES'],f['diagnostics'])
 n.expect('native-input-stream-disposed',not n.run('pactl','list','short','source-outputs').strip(),n.run('pactl','list','short','source-outputs'))
 selected=n.prefs();n.close_app();n.app('saved-devices-restart',preserve=True);n.tab(2);f=n.frame('device-selection-after-restart')
 n.expect('device-ids-survive-separate-process',all(n.prefs()[k]==selected[k] for k in ['microphone','speaker','camera']),{'before':selected,'after':n.prefs()})
 n.expect('restart-does-not-capture','session=None' in f['diagnostics']['UI_DEVICES'],f['diagnostics']);n.close_app()

def notification_checks(n):
 n.tab(3)
 # Native notification effects, observed independently of SubmittedToSystem.
 raw=n.out/'notification-enabled-sound.raw';sound=raw.open('wb');record=subprocess.Popen(['parec','--device=nmt_device_test.monitor','--raw','--format=s16le','--rate=48000','--channels=1','--latency-msec=20','--process-time-msec=20'],env=n.env,stdout=sound,stderr=subprocess.DEVNULL,start_new_session=True);n.jobs.append(record);time.sleep(.2)
 n.events.append({'pulse_before_notification':n.run('pactl','list','short','sinks'),'monitor_before_notification':n.run('pactl','list','short','source-outputs')});n.click_text('Testbenachrichtigung');time.sleep(3.6);f=n.frame('notification-enabled-native-banner',root=True)
 os.killpg(record.pid,signal.SIGTERM);record.wait(timeout=4);sound.close()
 import array
 samples=array.array('h');samples.frombytes(raw.read_bytes()[:96000*8]);peak=max(map(abs,samples),default=0)
 n.expect('notification-audio-observed-on-native-monitor',peak>100,{'samples':len(samples),'peak':peak,'sha256':sha(raw)})
 displayed=n.run('dunstctl','count','displayed').strip();n.expect('notification-daemon-displays-banner',int(displayed)>0,{'displayed':displayed})
 n.run('dunstctl','close-all');n.click(930,127);n.click(930,205);n.saved(sounds='0',banners='0')
 silent=n.out/'notification-suppressed-sound.raw';sound=silent.open('wb');record=subprocess.Popen(['parec','--device=nmt_device_test.monitor','--raw','--format=s16le','--rate=48000','--channels=1','--latency-msec=20','--process-time-msec=20'],env=n.env,stdout=sound,stderr=subprocess.DEVNULL,start_new_session=True);n.jobs.append(record);time.sleep(.3)
 n.click_text('Testbenachrichtigung');time.sleep(.7);f=n.frame('notification-policy-suppresses-effects')
 os.killpg(record.pid,signal.SIGTERM);record.wait(timeout=4);sound.close();samples=array.array('h');samples.frombytes(silent.read_bytes()[:96000*4]);peak=max(map(abs,samples),default=0)
 n.expect('suppressed-notification-no-banner-or-tone',len(samples)>4800 and peak==0 and n.run('dunstctl','count','displayed').strip()=='0',{'peak':peak,'samples':len(samples),'sha256':sha(silent),'ocr':f['ocr']})

def notifications(n):
 n.synthetic_audio();n.app('notification-settings');notification_checks(n);n.close_app()

def call_chat(n):
 n.a.width=720;n.a.height=580;n.app('call-chat',scenario='call-controls');n.click_text('Chat',dy=-36);n.frame('narrow-call-chat-open')
 # Keyboard-only entry from the real modal close control into its history/editor.
 for _ in range(8):
  n.key('Tab');d=n.diag()
  if 'editor_focused=true' in d['UI_LOCAL_STATE']:break
 n.expect('call-chat-editor-keyboard-reachable','editor_focused=true' in d['UI_LOCAL_STATE'] and 'overlay=Some(CallChat)' in d['UI_LOCAL_STATE'],d)
 n.type('ALPHA');n.key('shift+Return');n.type('BETA');f=n.frame('call-chat-native-multiline-input')
 n.expect('call-chat-native-input-retained','editor_chars=10 editor_focused=true' in f['diagnostics']['UI_LOCAL_STATE'],f['diagnostics'])
 n.key('Return');f=n.frame('call-chat-send-once-with-inline-result')
 intents=[line for line in n.app_log.read_text().splitlines() if 'UI_INTENT send ' in line]
 n.expect('call-chat-enter-single-send',len(intents)==1,{'intents':intents,'ocr':f['ocr']})
 n.run('xdotool','windowsize','--sync',n.xid,'1120','716');time.sleep(.5);f=n.frame('wide-resize-retains-one-overlay-editor')
 n.expect('resize-keeps-overlay-draft','overlay=Some(CallChat)' in f['diagnostics']['UI_LOCAL_STATE'] and 'editor_chars=10' in f['diagnostics']['UI_LOCAL_STATE'],f['diagnostics'])
 n.key('Escape');f=n.frame('close-call-chat-retains-call')
 n.expect('escape-never-leaves',not any('leave ' in s for s in n.app_log.read_text().splitlines() if 'UI_INTENT' in s),n.app_log.read_text()[-3000:])
 n.key('ctrl+comma');n.type('IGNORED');n.key('Escape');d=n.diag();n.expect('settings-excludes-call-composer','editor_chars=10' in d['UI_LOCAL_STATE'],d)
 n.close_app()

def notification_categories(n):
 n.synthetic_audio();n.app('category-settings');n.tab(3);n.frame('notifications-four-groups-initial')
 def action(label,activate=True):
  index=['Nachrichten testen','Wichtige Hinweise testen','Anrufhinweis testen','Kalenderhinweis testen'].index(label)
  for _ in range(32):
   d=n.diag()
   if 'focus_test=Some('+str(index)+')' in d['UI_NOTIFICATIONS']:
    n.events.append({'actual_keyboard_test_target':index,'label':label,'diagnostic':d['UI_NOTIFICATIONS']})
    if activate:n.key('Return')
    return
   n.key('Tab')
  raise AssertionError('Native keyboard Test focus not reached: '+label)
 def observed(label,name,enabled):
  import array
  n.run('dunstctl','close-all')
  raw=n.out/(name+'.raw');sound=raw.open('wb')
  record=subprocess.Popen(['parec','--device=nmt_device_test.monitor','--raw','--format=s16le','--rate=48000','--channels=1','--latency-msec=20','--process-time-msec=20'],env=n.env,stdout=sound,stderr=subprocess.DEVNULL,start_new_session=True);n.jobs.append(record);time.sleep(.2)
  action(label);time.sleep(1.2)
  # Observe before the helper's five-second expiry; OCR runs after capture.
  displayed=int(n.run('dunstctl','count','displayed').strip())
  f=n.frame(name,root=enabled,delay=.1)
  os.killpg(record.pid,signal.SIGTERM);record.wait(timeout=4);sound.close()
  samples=array.array('h');samples.frombytes(raw.read_bytes()[:96000*8]);peak=max(map(abs,samples),default=0)
  n.expect(name+'-actual-effects',len(samples)>4800 and ((peak>100 and displayed>0) if enabled else (peak==0 and displayed==0)),{'peak':peak,'samples':len(samples),'displayed':displayed,'sha256':sha(raw),'projection':f['diagnostics']['UI_NOTIFICATIONS']})
  n.run('dunstctl','close-all')
 observed('Nachrichten testen','messages-enabled-native',True)
 # The test button has native focus; traverse to its two preceding toggles.
 n.key('shift+Tab');n.key('shift+Tab');n.key('Return');n.key('Tab');n.key('Return')
 n.saved(messages_banners='0',messages_sounds='0')
 observed('Nachrichten testen','messages-disabled-native',False)
 observed('Wichtige Hinweise testen','important-independent-native',True)
 observed('Anrufhinweis testen','incoming-fallback-native',True)
 action('Kalenderhinweis testen',activate=False)
 f=n.frame('calendar-unavailable-and-opt-out');before=f['diagnostics']['UI_NOTIFICATIONS']
 n.expect('calendar-test-really-keyboard-focused','focus_test=Some(3)' in before,before)
 n.key('Return');n.key('space');time.sleep(.4);after=n.diag()['UI_NOTIFICATIONS']
 n.expect('calendar-without-source-never-submits',before==after and n.run('dunstctl','count','displayed').strip()=='0',{'before':before,'after':after})
 saved=n.prefs();n.close_app();n.app('categories-restart',preserve=True);n.tab(3);f=n.frame('notification-category-policy-after-restart')
 n.expect('independent-category-policy-survives-process',n.prefs()==saved and 'messages: NotificationDelivery { banners: false, sounds: false }' in f['diagnostics']['UI_NOTIFICATIONS'] and 'important: NotificationDelivery { banners: true, sounds: true }' in f['diagnostics']['UI_NOTIFICATIONS'] and 'calendar: NotificationDelivery { banners: false, sounds: false }' in f['diagnostics']['UI_NOTIFICATIONS'],f['diagnostics'])
 n.close_app()

def system_appearance(n):
 # Real Linux portal events on this recipe's private DBus/configuration only.
 n.env['XDG_CURRENT_DESKTOP']='GNOME'
 portal=Path(n.env['XDG_CONFIG_HOME'])/'xdg-desktop-portal';portal.mkdir();(portal/'portals.conf').write_text('[preferred]\ndefault=gtk\n')
 n.run('dbus-update-activation-environment','DISPLAY','XDG_CURRENT_DESKTOP','XDG_CONFIG_HOME','XDG_RUNTIME_DIR')
 for binary,service in [('xdg-permission-store','org.freedesktop.impl.portal.PermissionStore'),('xdg-document-portal','org.freedesktop.portal.Documents'),('xdg-desktop-portal-gtk','org.freedesktop.impl.portal.desktop.gtk'),('xdg-desktop-portal','org.freedesktop.portal.Desktop')]:
  n.spawn(binary,['/usr/libexec/'+binary]);n.run('gdbus','wait','--session','--timeout','10',service)
 def change(value,expected):
  n.run('gsettings','set','org.gnome.desktop.interface','color-scheme',value);time.sleep(.8)
  actual=n.run('gdbus','call','--session','--dest','org.freedesktop.portal.Desktop','--object-path','/org/freedesktop/portal/desktop','--method','org.freedesktop.portal.Settings.Read','org.freedesktop.appearance','color-scheme')
  n.expect('real-portal-value','uint32 '+str(expected) in actual,{'requested':value,'portal':actual})
 def pixel(frame):return n.run('convert',str(n.out/frame['path']),'-format','%[pixel:p{1090,400}]','info:').strip()
 change('prefer-light',2);n.app('system-appearance',scenario='chat',extra=['--ui-appearance','system']);n.saved(appearance='system')
 light=pixel(n.frame('system-preference-native-light'));change('prefer-dark',1);dark=pixel(n.frame('system-event-native-dark'))
 n.expect('saved-system-follows-native-appearance',light!=dark and n.prefs()['appearance']=='system',{'light':light,'dark':dark})
 n.key('ctrl+comma');n.tab(1);n.click(665,204);n.saved(appearance='light');n.key('Escape');fixed=pixel(n.frame('explicit-light-overrides-native-dark'))
 n.expect('explicit-appearance-overrides-observed-os',fixed==light and n.prefs()['appearance']=='light',{'fixed':fixed,'light':light});n.close_app()

def final_views(n):
 # Current gallery: ordinary no-account startup and real private computer writer.
 n.app('normal-settings',scenario=None);n.key('ctrl+comma');n.tab(0)
 n.click(930,228);n.saved(login='1');n.click(930,305);n.saved(background='1');n.frame('general-english-startup')
 n.click(625,166);n.saved(locale='de');n.frame('general-german-startup')
 n.click(712,166);n.saved(locale='es');n.frame('general-spanish-startup')
 n.click(625,166);n.saved(locale='de');n.tab(1);n.click(838,486);n.saved(sidebar='compact');n.frame('appearance-sidebar-compact')
 for tab,name in [(4,'privacy-no-admitted-account'),(5,'files-no-admitted-account')]:
  n.tab(tab);n.frame(name)
 n.close_app()
 # Only the conversation data below is fictional; preference values are loaded
 # by the same real writer in a separate process.
 n.app('sidebar-current',scenario='chat',preserve=True);n.frame('active-sidebar-compact-fictional-conversations');n.close_app()

def account_views(n):
 n.app('normal-account-settings',scenario=None);n.key('ctrl+comma');n.tab(0);n.click(625,166);n.saved(locale='de')
 for tab,name in [(4,'privacy-no-admitted-account'),(5,'files-no-admitted-account')]:
  n.tab(tab);f=n.frame(name);text=' '.join(f['ocr'].lower().split())
  n.expect('account-page-never-confirms-computer-save','noch kein konto verbunden.' in text and 'gespeichert' not in text,{'page':name,'raw_ocr':f['ocr'],'real_local_values':n.prefs()})
 n.close_app()

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--width',type=int,default=1120);p.add_argument('--height',type=int,default=716);p.add_argument('--mode',choices=['smoke','preferences','devices','call-chat','notifications','system','views','categories','account-views'],default='smoke');a=p.parse_args();n=Native(a)
 try:
  n.start()
  if a.mode=='preferences':preferences(n)
  elif a.mode=='devices':devices(n)
  elif a.mode=='call-chat':call_chat(n)
  elif a.mode=='notifications':notifications(n)
  elif a.mode=='system':system_appearance(n)
  elif a.mode=='views':final_views(n)
  elif a.mode=='categories':notification_categories(n)
  elif a.mode=='account-views':account_views(n)
  else:n.app('settings');n.frame('appearance-initial');n.tab(0);n.frame('general');n.tab(2);n.frame('devices');n.tab(3);n.frame('notifications');n.close_app()
 except BaseException as e:n.finish(repr(e));raise
 else:n.finish()
if __name__=='__main__':main()
