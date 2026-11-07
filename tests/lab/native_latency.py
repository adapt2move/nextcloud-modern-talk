#!/usr/bin/env python3
"""Native GPUI diagnostic workload against an owned released-server fixture.

No test executor or fake transport. Measures subprocess launch to native frame
content and UI intent admission to the after-frame observation. Xvfb/Mesa is a
Linux diagnostic profile, not macOS/GPU/physical display qualification.
"""
import argparse
import asyncio
import hashlib
import json
import os
import platform
from pathlib import Path
import select
import signal
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from urllib.parse import urlsplit

sys.dont_write_bytecode = True
from dev import owned_state, restore
from lab import save
from resilience import FaultRelay


async def process_run(binary, profile, env, mode, output, warm=False, performance=False, recovery=None):
    command = profile / "probe.command"
    command.unlink(missing_ok=True)
    started = time.monotonic_ns()
    process = await asyncio.create_subprocess_exec(str(binary), "--dev-profile", str(profile), "--dev-latency", env=env,
        stdout=asyncio.subprocess.PIPE, stderr=asyncio.subprocess.STDOUT, start_new_session=True)
    metrics, lines, acknowledgements = [], [], {}
    input_requests={}
    first_frame_colors=None
    stages=[]
    engine_lines = []
    perf, status, memory = [], [], []
    perf_done=set()
    initial_state=None
    phase="startup"
    current_action="startup"
    recovery_result=None
    async def read():
        while line := await process.stdout.readline():
            text = line.decode(errors="replace").strip()
            if len(engine_lines) < 256 and not text.startswith("DEV_"): engine_lines.append(text[:1024])
            if text.startswith("DEV_"):
                with (output/(mode+".trace")).open("a") as trace: trace.write(text+"\n")
                lines.append(text)
                if text.startswith("DEV_STAGE "): stages.append({"stage":text.split()[1],"elapsed_us":(time.monotonic_ns()-started)//1000})
                if text.startswith("DEV_PERF_DONE "): perf_done.add(text.split()[1])
                if text.startswith("DEV_PERF "):
                    _,work,interval,frame,messages,rooms=text.split()
                    perf.append({"work":work,"interval_us":int(interval),"frame_us":int(frame),"loaded_messages":int(messages),"rooms":int(rooms)})
                if text.startswith("DEV_STATUS "):
                    _,visible,queued,canonical,unique,sentinels,outgoing=text.split()
                    status.append({"visible":visible=="true","queued":int(queued),"canonical":int(canonical),"unique":int(unique),"sentinels":int(sentinels),"outgoing":int(outgoing)})
                if text.startswith("DEV_ACK "):
                    parts = text.split()
                    acknowledgements[int(parts[1])] = parts[2:]
                if text.startswith("DEV_INPUT "):
                    _,action,request=text.split();input_requests[action]=int(request)
                if text.startswith("DEV_METRIC "):
                    parts=text.split()
                    _, operation, micros, outcome = parts[:4]
                    request=int(parts[4]) if len(parts)>4 else None
                    metrics.append({"operation": operation, "elapsed_us": int(micros), "request":request, "before_input":not bool(input_requests), "outcome":outcome,"phase":phase,"action":current_action,
                        "launch_to_observation_us":(time.monotonic_ns()-started)//1000})
    reader = asyncio.create_task(read())
    sequence = 0
    async def act(action, timeout=10):
        nonlocal sequence, current_action
        sequence += 1
        if action not in ("state","status"): current_action=action
        next_file = profile / "probe.next"
        next_file.write_text(f"{sequence} {action}")
        next_file.replace(command)
        deadline = time.monotonic()+timeout
        while sequence not in acknowledgements:
            if process.returncode is not None: raise RuntimeError("Native process exited before acknowledgment")
            if time.monotonic()>deadline: raise RuntimeError("Native action acknowledgment timed out")
            await asyncio.sleep(.01)
        try:
            proc=Path(f"/proc/{process.pid}/status").read_text()
            memory.append({key:int(next(line.split()[1] for line in proc.splitlines() if line.startswith(key+":"))) for key in ("VmRSS","VmHWM")})
        except (OSError,StopIteration): pass
        return acknowledgements[sequence]
    async def ready(action="state"):
        deadline=time.monotonic()+150
        diagnosed=False
        if action != "state": await act(action)
        while time.monotonic()<deadline:
            state=await act("state")
            if len(state)==3 and int(state[0])>=2 and (action=="state" or int(state[1])>0) and state[2]=="ready": return state
            if not diagnosed and time.monotonic() >= deadline-140:
                await act("diagnose")
                diagnosed=True
            await asyncio.sleep(.2)
        await act("diagnose")
        raise RuntimeError("Native content did not become ready")
    try:
        deadline=time.monotonic()+90
        while time.monotonic()<deadline:
            found=await asyncio.to_thread(subprocess.run,["xdotool","search","--name","^Talk$"],text=True,capture_output=True,env=env)
            if found.returncode==0 and found.stdout.strip():
                xid=found.stdout.strip().splitlines()[0]
                if len(found.stdout.strip().splitlines()) != 1: raise RuntimeError("Ambiguous window on owned display")
                await asyncio.to_thread(subprocess.run,["xdotool","windowmap","--sync",xid],check=True,env=env)
                await asyncio.to_thread(subprocess.run,["xdotool","windowfocus","--sync",xid],check=True,env=env)
                break
            await asyncio.sleep(.1)
        else: raise RuntimeError("Native window did not map")
        initial_state=await ready()
        await asyncio.to_thread(subprocess.run,["import","-window",xid,str(output/(mode+"-initial-native.png"))],check=True,env=env,timeout=10)
        first_frame_colors=int(await asyncio.to_thread(subprocess.check_output,["identify","-format","%k",str(output/(mode+"-initial-native.png"))],env=env,text=True))
        phase="prepare"
        await ready("open0")
        await asyncio.sleep(.5)
        phase="switch"
        await ready("open1")
        await asyncio.sleep(.5)
        await ready("open0")
        phase="actions"
        if warm:
            await asyncio.sleep(3)
        elif not performance and not recovery:
            for action in ("send", "react", "read", "mark-read", "favorite"):
                expected = {"send":"send", "react":"reaction", "read":"read", "mark-read":"read", "favorite":"favorite"}[action]
                first=len(metrics)
                await act(action)
                request=input_requests.get(action)
                matches=lambda m: m["operation"]==expected and request not in (None,0) and m.get("request")==request
                deadline=time.monotonic()+32
                while not any(matches(m) for m in metrics[first:]) and time.monotonic()<deadline:
                    await asyncio.sleep(.01)
                for metric in metrics[first:]:
                    if matches(metric):metric["measured_action"]=action
                # Retain missing/failed observations; never silently drop samples.
                if not any(matches(m) for m in metrics[first:]): metrics.append({"operation":expected,"request":request,"measured_action":action,"outcome":"missing"})
                await asyncio.sleep(.2)
            await asyncio.sleep(1)
        if performance:
            deadline=time.monotonic()+150
            while int((await act("state"))[0]) != 200:
                if time.monotonic()>deadline: raise RuntimeError("Native room-list workload was truncated")
                await asyncio.sleep(.1)
            # Native Home also proves that the restored page can load older
            # history. Preparation is outside the frame timing intervals.
            await act("history-focus")
            await asyncio.to_thread(subprocess.run,["xdotool","key","--clearmodifiers","Home"],check=True,env=env)
            deadline=time.monotonic()+15
            while int((await act("state"))[1]) <= 50:
                if time.monotonic()>deadline:
                    await act("diagnose")
                    raise RuntimeError("Native Home did not load beyond the restored page")
                await asyncio.sleep(.1)
            for work in ("scroll","switcher","sidebar"):
                await act("perf-"+work)
                end=time.monotonic()+600
                while work not in perf_done and time.monotonic()<end: await asyncio.sleep(.05)
                if work not in perf_done: raise RuntimeError("Native performance workload timed out: "+work)
                completed=[s for s in perf if s['work']==work]
                if not completed or any(s['rooms'] != 200 or s['loaded_messages'] == 0 for s in completed): raise RuntimeError("Native workload lost its loaded room/message projection")
                if work == 'scroll' and not any(50 < s['loaded_messages'] <= 100 for s in completed): raise RuntimeError("Native scroll workload did not exercise bounded older-page loading")
                if sum(s['interval_us'] for s in completed)<60_000_000: raise RuntimeError("Native workload ended before its 60-second interval")
                await act("state")
            # Real native keyboard traversal across a culled offscreen row.
            # Preserve both stable controls, reveal, menu activation and scale.
            keyboard=[]
            for scale in (100,200):
                if scale == 200:
                    await act("sidebar-focus-large")
                    await asyncio.sleep(.5) # settle drawer layout/focus before targeting a row
                await act("sidebar-focus-tail")
                async def expect_focus(main,more,menu=False):
                    end=time.monotonic()+5
                    while time.monotonic()<end:
                        state=await act("sidebar-focus-state")
                        if state == [str(main).lower(),str(more).lower(),'true',str(menu).lower(),str(scale)]:
                            keyboard.append(state);return
                        await asyncio.sleep(.05)
                    raise RuntimeError("Native sidebar keyboard/reveal invariant failed: "+repr(state))
                async def key(value):
                    await asyncio.to_thread(subprocess.run,["xdotool","key", "--clearmodifiers",value],check=True,env=env)
                await expect_focus(True,False)
                await key("Tab");await expect_focus(False,True)
                await key("shift+Tab");await expect_focus(True,False)
                await key("Tab");await expect_focus(False,True)
                await key("Return")
                end=time.monotonic()+5
                while time.monotonic()<end:
                    state=await act("sidebar-focus-state")
                    if state[3]=='true': break
                    await asyncio.sleep(.05)
                else: raise RuntimeError("Native sidebar menu did not open")
                await key("Escape");await expect_focus(False,True)
                await asyncio.to_thread(subprocess.run,["import","-window",xid,str(output/(mode+f"-sidebar-keyboard-{scale}.png"))],check=True,env=env,timeout=10)
            save(output/(mode+"-sidebar-keyboard.json"),keyboard)
        if recovery:
            relay,control,lab,rooms=recovery
            await asyncio.sleep(5)
            control.write_text("disconnected");await relay.disconnect()
            # Observe a failed read before accepting the offline send. It must
            # remain queued without a publication attempt during the outage.
            await asyncio.sleep(8)
            await act("send");await asyncio.sleep(2);await act("status")
            offline=status[-1] if status else None
            subprocess.run(["import","-window",xid,str(output/(mode+"-offline-native.png"))],check=True,env=env,timeout=10)
            for room in rooms:
                lab.request("lab_bob","POST",f"/ocs/v2.php/apps/spreed/api/v1/chat/{room['token']}",body={"message":"Native recovery sentinel before suspend"},expected=(201,),name="native.recovery-before")
            os.killpg(process.pid,signal.SIGSTOP)
            try:
                await asyncio.sleep(61)
                for room in rooms:
                    lab.request("lab_bob","POST",f"/ocs/v2.php/apps/spreed/api/v1/chat/{room['token']}",body={"message":"Native recovery sentinel during suspend"},expected=(201,),name="native.recovery-during")
                control.write_text("healthy")
                resumed=time.monotonic_ns()
            finally: os.killpg(process.pid,signal.SIGCONT)
            end=time.monotonic()+90
            recovered=None
            while time.monotonic()<end:
                await act("status")
                if status and status[-1]['sentinels']==2 and not status[-1]['visible'] and status[-1]['outgoing']==0 and status[-1]['queued']==0:
                    recovered=status[-1];break
                await asyncio.sleep(.1)
            recovery_us=(time.monotonic_ns()-resumed)//1000
            # Distinct duplicate server IDs would pass the native ID check.
            # This independent read also proves the queued send was delivered,
            # rather than disappearing because its optimistic row was removed.
            delivered=0
            for room in rooms:
                latest=lab.request('lab_alice','GET',f"/ocs/v2.php/apps/spreed/api/v1/chat/{room['token']}",query={'lookIntoFuture':0,'limit':100,'setReadMarker':0},name='native.validate-delivery')
                delivered+=sum(m.get('message')=='Native latency fixture message' for m in latest)
            recovery_result={"offline":offline,"recovered":recovered,"server_deliveries":delivered,"suspend_seconds":61,"resume_to_catchup_us":recovery_us,"pass":bool(offline and offline['visible'] and offline['queued'] > 0 and offline['outgoing'] > 0 and recovered and recovered['queued']==0 and recovered['canonical']==recovered['unique'] and delivered==1)}
            subprocess.run(["import","-window",xid,str(output/(mode+"-recovered-native.png"))],check=True,env=env,timeout=10)
            save(output/(mode+"-recovery.json"),recovery_result)
            if not recovery_result['pass']: raise RuntimeError("Native recovery did not validate")
    finally:
        # SIGTERM is a crash-style exit: preserve the durable cache; no logout.
        try: os.killpg(process.pid, signal.SIGCONT)
        except ProcessLookupError: pass
        try: os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError: pass
        try: await asyncio.wait_for(process.wait(), 5)
        except TimeoutError:
            try: os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError: pass
            await process.wait()
        await reader
        save(output / (mode + ".json"), {"mode":mode,"warmup":warm,"stages":stages,"metrics":metrics,"trace":lines,"engine_log":engine_lines,"exit_code":process.returncode,"initial_state":initial_state,"first_frame_colors":first_frame_colors,"perf":perf,"memory_kib":memory,"status":status,"recovery":recovery_result})
    return metrics


async def exercise(args):
    task=asyncio.current_task()
    asyncio.get_running_loop().add_signal_handler(signal.SIGTERM, task.cancel)
    output=Path(args.output);output.mkdir(mode=0o700,parents=True,exist_ok=False)
    with owned_state(args.state) as (_, state):
        lab=restore(state)
        lab.deadline=time.monotonic()+1800
        binary=Path(args.binary).resolve()
        root=Path(tempfile.mkdtemp(prefix="nmt-native-",dir="/run"));os.chmod(root,0o700)
        profile=root/"profile";profile.mkdir(mode=0o700)
        existing=lab.request("lab_alice","GET","/ocs/v2.php/apps/spreed/api/v4/room",name="native.existing")
        for room in existing:
            if room.get("name","").startswith("A native latency"):
                lab.request("lab_alice","DELETE",f"/ocs/v2.php/apps/spreed/api/v4/room/{room['token']}",name="native.clean-existing")
        rooms=[]
        for n in range(2):
            room=lab.request("lab_alice","POST","/ocs/v2.php/apps/spreed/api/v4/room",body={"roomType":2,"roomName":f"A native latency {n}"},expected=(201,),name="native.create")
            rooms.append(room)
            lab.request("lab_alice","POST",f"/ocs/v2.php/apps/spreed/api/v4/room/{room['token']}/participants",body={"newParticipant":"lab_bob","source":"users"},name="native.member")
            for m in range(10):
                lab.request("lab_bob","POST",f"/ocs/v2.php/apps/spreed/api/v1/chat/{room['token']}",body={"message":f"Synthetic mixed fixture {m}: **bold** and a reply"},expected=(201,),name="native.message")
        workload=None
        if args.performance:
            from native_workload import seed
            workload=seed(lab,rooms)
            save(output/"workload.json",{key:value for key,value in workload.items() if key!='files'})
        def reset_flags():
            # Preparation is outside the timed child. Every case starts from
            # known read/favorite state, rather than inheriting an earlier intent.
            for room in rooms:
                token=room['token']
                latest=lab.request('lab_alice','GET',f'/ocs/v2.php/apps/spreed/api/v1/chat/{token}',query={'lookIntoFuture':0,'limit':1,'setReadMarker':0},name='native.prepare-latest')
                lab.request('lab_alice','POST',f'/ocs/v2.php/apps/spreed/api/v1/chat/{token}/read',body={'lastReadMessage':max(int(m['id']) for m in latest)},name='native.prepare-read')
                observed=lab.request('lab_alice','GET',f'/ocs/v2.php/apps/spreed/api/v4/room/{token}',name='native.prepare-room')
                if observed.get('isFavorite'):
                    lab.request('lab_alice','DELETE',f'/ocs/v2.php/apps/spreed/api/v4/room/{token}/favorite',name='native.prepare-favorite')
        reset_flags()
        def queued_records(cached_profile):
            files=list(cached_profile.glob('data-*.sqlite'))
            if len(files)!=1: raise RuntimeError('Expected one owned account cache')
            with sqlite3.connect(files[0].as_uri()+'?mode=ro',uri=True) as db:
                return db.execute('SELECT count(*) FROM outbox').fetchone()[0]
        fixture=root/"intake.json"
        save(fixture,{"origin":"http://127.0.0.1:47407/","user":"lab_alice","password":lab.passwords['lab_alice']})
        subprocess.run([str(binary),"--dev-seed",str(profile),str(fixture)],check=True,stdout=subprocess.DEVNULL)
        fixture.unlink()
        env=dict(os.environ)
        env.pop("WAYLAND_DISPLAY",None)
        env["VK_ICD_FILENAMES"]=str(sorted(Path('/usr/share/vulkan/icd.d').glob('lvp_icd*.json'))[0])
        for key,child in [("XDG_CONFIG_HOME","config"),("XDG_DATA_HOME","data"),("XDG_RUNTIME_DIR","runtime")]:
            path=root/child;path.mkdir(mode=0o700);env[key]=str(path)
        read_fd,write_fd=os.pipe()
        with (output/"xvfb.log").open('w') as log:
            xvfb=subprocess.Popen(["Xvfb","-displayfd",str(write_fd),"-screen","0","1440x900x24","-nolisten","tcp","-noreset"],pass_fds=(write_fd,),stdout=log,stderr=log)
            os.close(write_fd)
            try:
                if not select.select([read_fd],[],[],15)[0]: raise RuntimeError("Xvfb unavailable")
                env["DISPLAY"]=":"+os.read(read_fd,32).decode().strip()
                control=root/"fault.mode"
                async with FaultRelay(urlsplit(lab.base).hostname,47407,control) as relay:
                    for mode in ([] if args.recovery_only else (("healthy",) if args.performance else args.modes)):
                        for sample in range(args.samples):
                            control.write_text('healthy')
                            reset_flags()
                            case_profile=root/(mode+f'-{sample}-profile');case_profile.mkdir(mode=0o700)
                            save(fixture,{"origin":"http://127.0.0.1:47407/","user":"lab_alice","password":lab.passwords['lab_alice']})
                            subprocess.run([str(binary),"--dev-seed",str(case_profile),str(fixture)],check=True,stdout=subprocess.DEVNULL);fixture.unlink()
                            for attempt in range(3):
                                await process_run(binary,case_profile,env,mode+f'-{sample}-warmup-{attempt}',output,warm=True)
                                await relay.disconnect()
                                if queued_records(case_profile)==0: break
                            else: raise RuntimeError('Warm cache still contains unresolved fixture intents')
                            reset_flags()
                            control.write_text(mode)
                            await process_run(binary,case_profile,env,mode+f"-{sample}",output,performance=args.performance)
                            await relay.disconnect()
                            await asyncio.sleep(1)
                    if args.recovery or args.recovery_only:
                        control.write_text("healthy")
                        # Keep the suspend test independent from deliberately
                        # killed latency samples with unresolved dispatch markers.
                        recovery_profile=root/"recovery-profile";recovery_profile.mkdir(mode=0o700)
                        save(fixture,{"origin":"http://127.0.0.1:47407/","user":"lab_alice","password":lab.passwords['lab_alice']})
                        subprocess.run([str(binary),"--dev-seed",str(recovery_profile),str(fixture)],check=True,stdout=subprocess.DEVNULL);fixture.unlink()
                        reset_flags()
                        for attempt in range(3):
                            await process_run(binary,recovery_profile,env,"recovery-warmup-"+str(attempt),output,warm=True)
                            await relay.disconnect()
                            if queued_records(recovery_profile)==0: break
                        else: raise RuntimeError('Recovery warmup contains unresolved fixture intents')
                        await process_run(binary,recovery_profile,env,"recovery",output,recovery=(relay,control,lab,rooms))
                    save(output/"manifest.json",{"binary_sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),"samples_per_mode":args.samples,"cache_policy":"each measured process uses its own warmed owner-private cache with zero outgoing records; server read/favorite flags reset before warmup and cold launch","harness_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"environment":{"os":platform.platform(),"cpu":next((line.split(":",1)[1].strip() for line in Path('/proc/cpuinfo').read_text().splitlines() if line.startswith('model name')),'unknown'),"toolchain":subprocess.check_output(['rustc','--version'],text=True).strip(),"build":"ordinary release, dev-probes, RUSTFLAGS=-C debug-assertions=yes; no GPUI test-support"},"room_messages":10,"rooms_seeded":2,"renderer":"GPUI X11 / Mesa lavapipe","boundary":"native after-frame callback; startup includes process launch as observed by parent; not physical display presentation","injected":relay.injected})
            finally:
                os.close(read_fd);xvfb.terminate();xvfb.wait(timeout=5)
                import shutil
                shutil.rmtree(root)
                if workload:
                    from native_workload import cleanup
                    cleanup(lab,workload)

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--modes',nargs='+',choices=['healthy','latency-1500','latency-4000'],default=['healthy','latency-1500','latency-4000'])
    parser.add_argument('--samples',type=int,choices=range(1,6),default=2)
    parser.add_argument('--performance',action='store_true')
    parser.add_argument('--recovery',action='store_true')
    parser.add_argument('--recovery-only',action='store_true')
    parser.add_argument('--state',required=True);parser.add_argument('--binary',required=True);parser.add_argument('--output',required=True)
    asyncio.run(asyncio.wait_for(exercise(parser.parse_args()),3600))
