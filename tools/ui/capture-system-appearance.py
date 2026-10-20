#!/usr/bin/env python3
"""Linux native System appearance through real XDG portal/GSettings events.

Run under dbus-run-session on the author VM. Uses a disposable user configuration,
owned portal processes and Xvfb; does not simulate app state or qualify macOS.
"""
import argparse, importlib.util, json, os, subprocess, sys, tempfile, time
from pathlib import Path
p=argparse.ArgumentParser(description=__doc__);p.add_argument('--build-input',type=Path,required=True);p.add_argument('--output',type=Path,required=True)
a=p.parse_args();base=a.build_input.resolve();out=a.output.resolve();out.mkdir(parents=True,exist_ok=False)
sys.path.insert(0,str(base/'source/tools/ui'))
import capture,interactions_d1_2
spec=importlib.util.spec_from_file_location('frozen',base/'source/tools/ui/capture-frozen.py');frozen=importlib.util.module_from_spec(spec);spec.loader.exec_module(frozen)
binding,_=frozen.validate(base);original_run=capture.run;processes=[];events=[];frames=[];checks=[]
def portal_value(env):
    return original_run(['gdbus','call','--session','--dest','org.freedesktop.portal.Desktop','--object-path','/org/freedesktop/portal/desktop','--method','org.freedesktop.portal.Settings.Read','org.freedesktop.appearance','color-scheme'],env).stdout.strip()
def run(argv,env,**kwargs):
    if argv[0]=='vulkaninfo':
        original_run(['dbus-update-activation-environment','DISPLAY','XDG_CURRENT_DESKTOP','XDG_CONFIG_HOME','XDG_RUNTIME_DIR'],env)
        log=open(out/'portal.log','w')
        for binary,service in [('/usr/libexec/xdg-permission-store','org.freedesktop.impl.portal.PermissionStore'),('/usr/libexec/xdg-document-portal','org.freedesktop.portal.Documents'),('/usr/libexec/xdg-desktop-portal-gtk','org.freedesktop.impl.portal.desktop.gtk'),('/usr/libexec/xdg-desktop-portal','org.freedesktop.portal.Desktop')]:
            processes.append(subprocess.Popen([binary],env=env,stdout=log,stderr=log))
            original_run(['gdbus','wait','--session','--timeout','10',service],env)
        time.sleep(.5)
        original_run(['gsettings','set','org.gnome.desktop.interface','color-scheme','prefer-light'],env)
        time.sleep(.5)
        initial=portal_value(env);events.append({'initial_portal':initial})
        if 'uint32 2' not in initial: raise RuntimeError('Disposable portal backend did not observe initial light preference: '+initial)
    return original_run(argv,env,**kwargs)
capture.run=run
def exercise(scenario,env,xid,folder,label,large=False):
    def key(k):original_run(['xdotool','key','--clearmodifiers',k],env);time.sleep(.25)
    def set_os(value):
        original_run(['gsettings','set','org.gnome.desktop.interface','color-scheme',value],env);time.sleep(1)
        events.append({'gsettings':value,'gsettings_read':original_run(['gsettings','get','org.gnome.desktop.interface','color-scheme'],env).stdout,'portal':portal_value(env)})
    def frame(name):
        q=folder/f'{name}.png';original_run(['import','-window',xid,str(q)],env)
        color=original_run(['convert',str(q),'-format','%[pixel:p{1090,400}]','info:'],env).stdout
        frames.append({'name':name,'image':q.name,'sha256':capture.sha(q),'surface_pixel':color,'events':events.copy()});return color
    light=frame('system-initial-light');set_os('prefer-dark');dark=frame('system-event-dark')
    checks.append({'check':'System-follows-real-portal-event','result':'pass' if dark!=light else 'FAIL','light':light,'dark':dark})
    set_os('prefer-light');again=frame('system-event-light')
    checks.append({'check':'System-returns-to-light','result':'pass' if again==light else 'FAIL'})
    key('ctrl+comma');key('Right');key('Return');key('Escape');frame('explicit-light-selected')
    set_os('prefer-dark');fixed=frame('explicit-light-ignores-os-dark')
    checks.append({'check':'explicit-Light-overrides-OS','result':'pass' if fixed==light else 'FAIL'})
    (out/'appearance-observations.json').write_text(json.dumps({'frames':frames,'events':events,'checks':checks,'scope':'Linux portal integration only; no macOS, AX or preference persistence claim'},indent=2)+'\n')
    if any(c['result']=='FAIL' for c in checks):raise AssertionError(checks)
    return checks
def cleanup(env):
    for process in reversed(processes):
        if process.poll() is None:process.terminate()
        try:process.wait(timeout=3)
        except subprocess.TimeoutExpired:process.kill();process.wait(timeout=3)
    mount=Path(env['XDG_RUNTIME_DIR'])/'doc'
    if os.path.ismount(mount):original_run(['fusermount3','-u',str(mount)],env)
    result={'owned_portals_stopped':all(p.poll() is not None for p in processes),'private_document_mount_absent':not os.path.ismount(mount)}
    events.append(result)
    (out/'owned-cleanup.json').write_text(json.dumps(result,indent=2)+'\n')
    if not all(result.values()):raise RuntimeError('Owned portal cleanup incomplete')
frozen.capture=lambda args:capture.capture(args,before_runtime_cleanup=cleanup)
interactions_d1_2.exercise=exercise
with tempfile.TemporaryDirectory(prefix='talk-ui-theme-config-') as config:
    os.environ['XDG_CONFIG_HOME']=config;os.environ['XDG_CURRENT_DESKTOP']='GNOME'
    portal=Path(config)/'xdg-desktop-portal';portal.mkdir();(portal/'portals.conf').write_text('[preferred]\ndefault=gtk\n')
    sys.argv=['capture-frozen.py','--build-input',str(base),'--output',str(out),'--width','1120','--height','716','--scenarios','chat:system:blue','--interactions','--interaction-suite','d1-2']
    try:frozen.main()
    finally:
        for process in reversed(processes):
            if process.poll() is None:process.terminate()
            try:process.wait(timeout=3)
            except subprocess.TimeoutExpired:process.kill();process.wait(timeout=3)
        frozen.validate(base)
        (out/'appearance-recipe-binding.json').write_text(json.dumps({'recipe_sha256':capture.sha(__file__),'compiled_input':binding,'real_portal':True,'owned_processes_closed':True},indent=2)+'\n')
