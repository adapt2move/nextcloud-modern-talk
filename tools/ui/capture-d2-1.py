#!/usr/bin/env python3
"""Targeted D2.1 native input evidence, VM only; no application-state injection.

Every assertion records actual input/geometry/counts/dispatch and full raw OCR.
Original window pixels are never edited. Local privacy fixtures do not qualify
bounded platform clipboard ingress. Historical failed probes remain separate.
"""
import argparse,importlib.util,json,re,subprocess,sys,time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__)
p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
p.add_argument('--batch',choices=['history','search','hints','privacy','lifecycle','calls','prejoin','surfaces','hover','editor','isolation','media','hover-lifecycle','download'],required=True)
p.add_argument('--scenarios',required=True);p.add_argument('--scale',type=int,default=100)
p.add_argument('--width',type=int,default=1120);p.add_argument('--height',type=int,default=716)
a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve();sys.path.insert(0,str(base/'source/tools/ui'))
from capture import run,sha
import interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py');f=importlib.util.module_from_spec(spec);spec.loader.exec_module(f);binding,_=f.validate(base)
def exercise(scenario,env,xid,folder,label,large=False):
 events=[];frames=[];checks=[];log=folder/(label+'.log')
 def key(v,n=1):
  for _ in range(n):run(['xdotool','key','--clearmodifiers',v],env);events.append({'key':v});time.sleep(.14)
 def move(x,y):run(['xdotool','mousemove','--window',xid,str(x),str(y)],env);events.append({'pointer':[x,y]});time.sleep(.7)
 def click(x,y):move(x,y);run(['xdotool','click','1'],env);events.append({'click':[x,y]});time.sleep(.25)
 def text(v):run(['xdotool','type','--clearmodifiers','--delay','5',v],env);events.append({'synthetic_text':v});time.sleep(.2)
 def diag():
  before=log.read_text().count('UI_GEOMETRY ')
  key('F7');deadline=time.monotonic()+15
  while log.read_text().count('UI_GEOMETRY ')<=before:
   if time.monotonic()>deadline:raise AssertionError('Native diagnostic acknowledgement timed out')
   time.sleep(.05)
  time.sleep(.7 if a.scale<200 else 1.2);lines=log.read_text().splitlines()
  def last(prefix):return next((v for v in reversed(lines) if v.startswith(prefix)), '')
  return {'state':last('UI_LOCAL_STATE'),'history':last('UI_HISTORY'),'geometry':last('UI_GEOMETRY'),'action_focus':last('UI_ACTION_FOCUS'),'hover':last('UI_HOVER')}
 def dispatch():return [v for v in log.read_text().splitlines() if v.startswith(('UI_DISPATCH','UI_INTENT','UI_FOCUS_PROBE'))]
 def focus_until(predicate,limit=30):
  for _ in range(limit):
   key('Tab');d=diag()
   if predicate(d):return d
  raise AssertionError('Expected keyboard target not reached: '+str(d))
 def point(d):
  m=re.search(r'focus=Some\(Bounds \{ origin: Point \{ x: ([-\d.]+)px, y: ([-\d.]+)px \}, size: Size \{ ([-\d.]+)px × ([-\d.]+)px',d['geometry'])
  return tuple(map(float,m.groups())) if m else None
 def image(name,expected=None):
  time.sleep(.6);d=diag();q=folder/(label+'-d21-'+name+'.png');run(['import','-window',xid,str(q)],env)
  raw=run(['tesseract',str(q),'stdout','-l','deu+eng','--psm','11'],env).stdout
  (folder/(q.stem+'-raw-ocr.txt')).write_text(raw)
  frames.append({'name':name,'image':q.name,'sha256':sha(q),'events':events.copy(),'diagnostics':d,'dispatch':dispatch(),'raw_ocr':raw,'full_expected_copy_for_visual_inspection':expected})
  return frames[-1]
 def check(name,condition,evidence):
  checks.append({'check':name,'result':'pass' if condition else 'FAIL','evidence':evidence})
  if not condition:raise AssertionError(name+': '+str(evidence))
 def editor():return focus_until(lambda d:'editor_focused=true' in d['state'])
 def history():return focus_until(lambda d:'history_focused=true' in d['state'])
 def offset(d):
  m=re.search(r'y: ([-\d.]+)px.*× ([-\d.]+)px',d['history']);return tuple(map(float,m.groups())) if m else None
 def copy_draft():
  key('ctrl+a');key('ctrl+c');v=run(['xclip','-selection','clipboard','-o'],env).stdout;key('End');return v
 def outside():move(a.width-5,a.height-5)
 try:
  outside();initial=image('initial')
  if a.batch=='history':
   d=diag();o=offset(d);check('initial-newest-loaded-message',o is not None and abs(o[0]+o[1])<=2,o)
   history();key('Home');old=image('keyboard-oldest');o=offset(old['diagnostics']);check('Home-reaches-older-loaded-history',o is not None and abs(o[0])<=2,o)
   key('Next');middle=image('keyboard-page-down');o=offset(middle['diagnostics']);check('PageDown-moves-history',o is not None and o[0]<-2,o)
   editor();text('ALPHA');key('shift+Return');text('BRAVO');key('shift+Return');text('CHARLIE');older=image('older-reading-position-retained')
   check('three-lines-actually-entered','editor_chars=19' in older['diagnostics']['state'],older['diagnostics']);n=offset(older['diagnostics']);check('older-position-survives-composer-growth',n is not None and o is not None and abs(n[0]-o[0])<3,(o,n))
   history();key('End');latest=image('keyboard-latest');o=offset(latest['diagnostics']);check('End-reaches-latest',o is not None and abs(o[0]+o[1])<3,o)
   editor();key('End');key('shift+Return');text('DELTA');key('shift+Return');text('ECHO');key('shift+Return');text('FOXTROT');key('shift+Return');text('GOLF');seven=image('seven-lines-editor-caret-visible');check('seven-lines-actually-entered','editor_chars=43' in seven['diagnostics']['state'],seven['diagnostics'])
   key('Return');result=image('send-result');o=offset(result['diagnostics']);check('latest-retained-through-result-and-growth',o is not None and abs(o[0]+o[1])<3,o)
   check('draft-seven-lines-retained',copy_draft()=='ALPHA\nBRAVO\nCHARLIE\nDELTA\nECHO\nFOXTROT\nGOLF','native synthetic clipboard read')
   sends=[l for l in dispatch() if l.startswith('UI_INTENT send')];check('one-intent-per-send-Enter',len(sends)==1,sends)
   if scenario.endswith(('pending','uncertain')):
    key('Return');check('no-blind-resend',len([l for l in dispatch() if l.startswith('UI_INTENT send')])==1,dispatch())
  elif a.batch=='search':
   baseline=diag();key('ctrl+k');key('ctrl+a');text('Navigation');image('query-with-sidebar-preserved');key('Escape');ret=image('Escape-exact-filter-restored')
   check('modal-search-does-not-change-sidebar-filter','filter_chars=0' in ret['diagnostics']['state'],ret['diagnostics'])
   comparison=subprocess.run(['compare','-metric','AE',str(folder/initial['image']),str(folder/ret['image']),'null:'],env=env,capture_output=True,text=True)
   check('Escape-restores-native-baseline-pixels',comparison.returncode==0,comparison.stderr)
   key('ctrl+k');key('ctrl+a');text('Navigation')
   focus_until(lambda d: (b:=point(d)) is not None and b[2]>400 and b[1]>280 and b[1]<500)
   image('zero-reply-message-hit-focused');key('Return');hit=image('zero-reply-message-located')
   check('zero-reply-hit-locates-message-not-thread','overlay=None' in hit['diagnostics']['state'] and 'highlighted=Some(1)' in hit['diagnostics']['state'],hit['diagnostics'])
   key('ctrl+k');key('ctrl+a');text('Tastenk');image('positive-reply-message-result');d=focus_until(lambda d:(b:=point(d)) is not None and b[2]>400 and b[1]>280 and b[1]<550);b=point(d);key('shift+Tab',2);click(int(b[0]+b[2]/2),int(b[1]+b[3]/2));hit=image('pointer-message-located')
   check('pointer-hit-locates-message-not-thread','overlay=None' in hit['diagnostics']['state'] and 'highlighted=Some(2)' in hit['diagnostics']['state'],hit['diagnostics'])
   key('ctrl+k');key('ctrl+a');text('Produktteam');key('Escape');ret=image('conversation-query-Escape-restores-list');check('conversation-query-filter-unchanged','filter_chars=0' in ret['diagnostics']['state'],ret['diagnostics'])
   click(115,130);text('Mara');pre=image('genuine-sidebar-filter');key('ctrl+k');key('ctrl+a');text('Navigation');key('Escape');post=image('genuine-filter-restored');check('nonempty-sidebar-filter-retained','filter_chars=4' in post['diagnostics']['state'],post['diagnostics'])
   click(240,130);cleared=image('sidebar-filter-cleared');check('genuine-filter-can-be-cleared','filter_chars=0' in cleared['diagnostics']['state'],cleared['diagnostics'])
  elif a.batch=='hints':
   if scenario=='offline':
    x=a.width-100;y=50 if a.scale==100 else 110
    move(x,y);image('header-call-hover-700ms','Anruf beitreten\nDu bist offline.');outside()
    d=focus_until(lambda d:'Some("call")' in d['action_focus'])
    before=image('header-call-focus');b=point(d);move(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('header-call-focus-hover-700ms');original=dispatch();key('Return');key('space');click(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('disabled-pointer-enter-space');check('disabled-call-no-dispatch',dispatch()==original,dispatch())
    move(a.width-54,y);image('details-hover-700ms','Gesprächsdetails')
   else:
    editor();text('ENTWURF');key('Return');image('send-pending-or-uncertain');focus_until(lambda d:'Some("send")' in d['action_focus']);focused=image('send-keyboard-reason');b=point(focused['diagnostics']);check('send-control-focused',b is not None and b[0]>a.width-120,b)
    if b:move(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('send-focus-hover-700ms')
  elif a.batch=='privacy':
   editor();text('ALPHA');start=image('keyboard-composer-ALPHA');check('nonempty-draft-before-privacy-check','editor_chars=5' in start['diagnostics']['state'] and 'editor_focused=true' in start['diagnostics']['state'],start['diagnostics'])
   sentinel=folder/(label+'-clipboard-sentinel.txt');sentinel.write_text('PRIVATE-SENTINEL')
   owner=subprocess.Popen(['xclip','-selection','clipboard','-i',str(sentinel)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   try:
    time.sleep(.2);key('ctrl+a');key('ctrl+c');copy=image('copy-refused','Kopieren ist für diesen Entwurf nicht freigegeben.');v=run(['xclip','-selection','clipboard','-o'],env).stdout
    check('copy-policy-preserves-existing-clipboard',v=='PRIVATE-SENTINEL',v);key('End');text('Z');typed=image('typed-Z-after-refusal');check('visible-draft-survives-refusal','editor_chars=6' in typed['diagnostics']['state'],typed['diagnostics'])
    key('ctrl+v');paste=image('paste-without-export-authority','ALPHAZPRIVATE-SENTINEL');check('paste-imports-without-export-authority',f"editor_chars={len('ALPHAZPRIVATE-SENTINEL')}" in paste['diagnostics']['state'],paste['diagnostics'])
    check('paste-keeps-existing-clipboard',run(['xclip','-selection','clipboard','-o'],env).stdout=='PRIVATE-SENTINEL','synthetic clipboard owner')
    key('ctrl+z');undone=image('paste-undo-restores-draft');check('paste-is-one-undoable-edit','editor_chars=6' in undone['diagnostics']['state'],undone['diagnostics'])
   finally:owner.terminate();owner.wait(timeout=2)
  elif a.batch=='lifecycle':
   if scenario=='server':key('Tab',2);key('Return');image('login-admitted-projection-no-global-progress')
   elif scenario=='incoming':
    original=dispatch();key('Return');check('unfocused-Enter-never-accepts',dispatch()==original,dispatch());key('Tab',2);image('accept-visible-consequences');key('Return');image('accepted-direct-no-generic-progress');check('direct-accept-once-without-conference',len([v for v in dispatch() if v.startswith('UI_INTENT accept-direct')])==1 and not any('join-conference' in v for v in dispatch()),dispatch());original=dispatch();key('Escape');check('Escape-never-hangs-up',dispatch()==original,dispatch())
   else:
    click(110,410);image('pointer-conversation-opened');check('pointer-opens-Mara-once',len([v for v in dispatch() if v.endswith('open account=1 room=3')])==1,dispatch());focus_until(lambda d:(b:=point(d)) is not None and b[0]<270 and b[1]>200 and b[2]>200);key('Return');image('keyboard-conversation-opened');check('two-navigation-intents-only',len([v for v in dispatch() if v.startswith(('UI_INTENT open','UI_FOCUS_PROBE open'))])==2,dispatch())
  elif a.batch=='prejoin':
   if scenario in ['chat','voice-room']:
    invoker=focus_until(lambda d:'Some("call")' in d['action_focus']);key('Return');image('keyboard-prejoin-initial-title-visible')
   else:image('prejoin-initial-title-visible')
   original=dispatch();key('ctrl+comma');key('Escape');image('settings-returns-prejoin');key('Escape');back=image('prejoin-returns-invoker');check('prejoin-returns-actual-header-call',scenario not in ['chat','voice-room'] or 'Some("call")' in back['diagnostics']['action_focus'],back['diagnostics']);check('prejoin-settings-Escape-no-join',not any('join-conference' in v for v in dispatch()),dispatch())
   if scenario in ['chat','voice-room']:
    click(a.width-100,50 if a.scale==100 else 110);outside();image('pointer-prejoin-initial-title-visible')
    focus_until(lambda d:'Some("join-prepared")' in d['action_focus'])
    image('voluntary-join-focused');key('Return');joined=image('voluntary-join-connecting')
    check('one-voluntary-join-only',len([v for v in dispatch() if v.startswith('UI_INTENT join-conference')])==1,dispatch())
    original=dispatch();key('Escape');check('Escape-after-join-never-leaves',dispatch()==original,dispatch())
  elif a.batch=='calls':
   key('Tab');image('first-action-focused');original=dispatch()
   if scenario=='call-error-unknown':
    d=focus_until(lambda d:'Some("rejoin-call")' in d['action_focus']);b=point(d);image('unknown-rejoin-reason');key('Return');key('space');click(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('unknown-rejoin-inert');check('unknown-rejoin-no-dispatch',dispatch()==original,dispatch())
   key('Escape');image('Escape-retains-outcome');check('Escape-no-leave',dispatch()==original,dispatch())
   if scenario in ['call-ended','call-error','call-error-unknown']:
    key('Tab');key('Return');ret=image('terminal-return-to-conversation');check('terminal-return-is-local-no-leave',dispatch()==original and 'Anruf beendet' not in ret['raw_ocr'] and 'Verbindung fehlgeschlagen' not in ret['raw_ocr'],{'dispatch':dispatch(),'raw_ocr':ret['raw_ocr']})
  elif a.batch=='editor':
   editor();text('ALPHAOMEGA');key('Home');key('Right',5);key('Tab');d=diag();b=point(d);image('attachment-keyboard-reason')
   check('attachment-actual-focus','Some("attach")' in d['action_focus'],d)
   key('shift+Tab');move(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('attachment-hover-editor-retained')
   click(int(b[0]+b[2]/2),int(b[1]+b[3]/2));text('X');typed=image('typing-dismisses-hover');check('disabled-click-keeps-caret',copy_draft()=='ALPHAXOMEGA','native nonempty draft')
   key('Home');key('Right',6);key('ctrl+comma');modal=image('settings-from-caret');click(70,400);text('MODALCANARY');key('ctrl+k');blocked=image('modal-background-inert')
   check('modal-native-background-inert',modal['sha256']==blocked['sha256'],[modal['sha256'],blocked['sha256']]);key('Escape');text('Y');image('modal-restores-caret');check('caret-restored',copy_draft()=='ALPHAXYOMEGA','native nonempty draft')
   d=diag();card=point(d);key('Escape');click(int(card[0]+card[2]/2),int(card[1]+card[3]-16));text('Z');image('card-padding-focuses-editor');actual=copy_draft();check('whole-card-focuses-editor',actual=='ALPHAXYOMEGAZ',{'synthetic_draft':actual})
   payload=folder/(label+'-oversize.txt');payload.write_text('Q'*4097)
   owner=subprocess.Popen(['xclip','-selection','clipboard','-i',str(payload)],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
   try:
    time.sleep(.2);key('ctrl+v');rejected=image('oversize-paste-outer-card-rejection','Der eingefügte Text ist zu lang. Dein Entwurf bleibt erhalten.');check('oversize-preserves-real-draft','editor_chars=13' in rejected['diagnostics']['state'],rejected['diagnostics'])
   finally:owner.terminate();owner.wait(timeout=2)
  elif a.batch=='isolation':
   key('Tab',7);image('Mara-focused');key('F6');image('Mara-reordered');key('Return');image('same-scoped-room-opened')
   check('reorder-opens-original-room-once',[v for v in dispatch() if v.startswith('UI_FOCUS_PROBE open')]==['UI_FOCUS_PROBE open account=1 room=3'],dispatch())
   key('F6');image('focused-room-removed');original=dispatch();key('Return');key('space');check('removed-room-no-redirect',dispatch()==original,dispatch())
   key('Tab',7);key('F6');image('account-runtime-replaced');original=dispatch();key('Return');key('space');check('account-runtime-no-redirect',dispatch()==original,dispatch())
  elif a.batch=='media':
   focus_until(lambda d:'Some("microphone")' in d['action_focus']);mic=image('microphone-state-focused');b=point(mic['diagnostics']);move(int(b[0]+b[2]/2),int(b[1]+b[3]/2));image('microphone-state-focus-hover');original=dispatch();key('Return');key('space');image('microphone-activation')
   if scenario in ['call-controls','call-active']:check('media-one-intent-per-key',len([v for v in dispatch() if v.startswith('UI_INTENT microphone')])==2,dispatch())
   else:check('unavailable-media-no-dispatch',dispatch()==original,dispatch())
   focus_until(lambda d:'Some("camera")' in d['action_focus']);image('camera-state-focused');focus_until(lambda d:'Some("leave-call")' in d['action_focus']);image('priority-leave-visible');original=dispatch();key('Escape');check('Escape-never-leaves',dispatch()==original,dispatch())
  elif a.batch=='surfaces':
   # Start in workspace; every surface is opened by its real visible control.
   for name,x,y in [('details',a.width-54,50),('create',240,87),('discover',130,600)]:
    click(x,y);outside();opened=image(name+'-opened');key('Escape');closed=image(name+'-Escape-workspace')
    check(name+'-opened-and-closed', 'overlay=Some' in opened['diagnostics']['state'] and 'overlay=None' in closed['diagnostics']['state'] and 'filter_chars=0' in closed['diagnostics']['state'],[opened['diagnostics'],closed['diagnostics']])
  elif a.batch=='download':
   d=focus_until(lambda d:(b:=point(d)) is not None and b[2]>400 and 50<b[3]<160 and b[1]>160 and 'editor_focused=false' in d['state'])
   focused=image('download-actual-focus');original=dispatch();key('Return');result=image('download-action-specific-result','Herunterladen ist in diesem Client noch nicht verfügbar.')
   check('download-single-intent',len([v for v in dispatch() if v.startswith('UI_FOCUS_PROBE download')])==1,dispatch())
   check('download-result-has-action', 'Herunterladen' in result['raw_ocr'],result['raw_ocr']);admitted=dispatch();key('Escape');returned=image('download-Escape-restores-file');check('download-Escape-no-new-intent',dispatch()==admitted,dispatch());check('download-Escape-same-file-focus',point(returned['diagnostics'])==point(focused['diagnostics']),[point(focused['diagnostics']),point(returned['diagnostics'])])
  elif a.batch=='hover-lifecycle':
   # A direct F7 acknowledgement deliberately avoids the screenshot settling
   # delay: observe the real pending interval, then two separated idle samples.
   observations=[]
   def hp(name):
    count=log.read_text().count('UI_HOVER ');run(['xdotool','key','F7'],env);deadline=time.monotonic()+5
    while log.read_text().count('UI_HOVER ')<=count:
     if time.monotonic()>deadline:raise AssertionError('hover observation timed out')
     time.sleep(.005)
    line=next(v for v in reversed(log.read_text().splitlines()) if v.startswith('UI_HOVER '));m=re.search(r'\((true|false), (true|false), (\d+)\)',line);v=(m[1]=='true',m[2]=='true',int(m[3]));observations.append({'name':name,'monotonic':time.monotonic(),'value':v});return v
   def raw_move(x,y):run(['xdotool','mousemove','--window',xid,str(x),str(y)],env);events.append({'pointer':[x,y],'wait_ms':80});time.sleep(.08)
   def idle(name,shown=False):
    time.sleep(.7);one=hp(name+'-1');time.sleep(.7);two=hp(name+'-2');check(name+'-no-deadline-frames',one==two and one[:2]==(False,shown),[one,two]);return two
   idle('initial-idle');raw_move(a.width-54,50);pending=hp('details-pending');check('one-current-hint-is-pending',pending[0] and not pending[1],pending);idle('displayed',True);image('hover-lifecycle-displayed-name','Gesprächsdetails')
   raw_move(a.width-5,a.height-5);idle('pointer-exit');raw_move(a.width-54,50);hp('before-input');text('X');idle('typing-cancels')
   raw_move(a.width-5,a.height-5);raw_move(a.width-54,50);hp('before-modal');key('ctrl+comma');idle('modal-change-cancels');key('Escape');idle('modal-return-no-stale-hint')
   # A row has hover styling, but no name/reason tooltip, so no deadline.
   prior=hp('before-row');raw_move(110,520);after=idle('row-without-hint');check('non-hint-row-never-schedules',after[2]==prior[2],[prior,after])
   if scenario=='focus-rooms':
    for phase in ['reorder','remove','account-runtime']:
     raw_move(a.width-5,a.height-5);raw_move(a.width-54,50);hp('before-'+phase);key('F6');idle(phase+'-invalidates-stale-target')
   else:
    raw_move(a.width-5,a.height-5);raw_move(a.width-54,50);hp('before-retirement');run(['xdotool','mousemove','--window',xid,str(a.width-100),'50','click','1'],env);events.append({'click':[a.width-100,50]});idle('workspace-retirement');image('retired-view-no-delayed-hint')
   (folder/(label+'-hover-lifecycle.json')).write_text(json.dumps({'observations':observations,'requests_are_only_this_hint_owners_requests':True,'not_process_performance_qualification':True},indent=2)+'\n')
  elif a.batch=='hover':
   # Non-selected Jonas remains independent from Produktteam selection.
   move(110,520);image('Jonas-hover-selection-retained');outside();image('Jonas-unhovered-selection-retained')
   check('hover-never-dispatches',not dispatch(),dispatch())
  return checks
 finally:
  (folder/(label+'-d21-observations.json')).write_text(json.dumps({'batch':a.batch,'frames':frames,'checks':checks,'hover_wait_ms':700,'wheel_events':0},indent=2)+'\n')
interactions_d1_2.exercise=exercise
sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--scenarios',a.scenarios,'--scale',str(a.scale),'--width',str(a.width),'--height',str(a.height),'--interactions','--interaction-suite','d1-2']
try:f.main()
finally:
 f.validate(base)
 (out/'d21-recipe-binding.json').write_text(json.dumps({'recipe_sha256':sha(__file__),'compiled_input':binding,'source_unchanged':True,'local_projection_only':True},indent=2)+'\n')
