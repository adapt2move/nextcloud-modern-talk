#!/usr/bin/env python3
"""Disposable source-bound protocol checks; synthetic credentials stay under /run."""
import argparse,asyncio,hashlib,ipaddress,json,os,pathlib,shutil,subprocess,sys,tempfile,time,urllib.parse
sys.dont_write_bytecode=True
from dev import owned_state,restore,private_json
class Relay:
    def __init__(self, host, port):
        address=ipaddress.ip_address(host)
        if not address.is_private or address.is_loopback or not 1024<=port<=65535:
            raise RuntimeError('isolated bridge only')
        self.host,self.port=str(address),port
        self.tasks=set()
    async def __aenter__(self):
        self.server=await asyncio.start_server(self.connection,'127.0.0.1',self.port,limit=65536,backlog=8)
        return self
    async def pipe(self,reader,writer):
        while True:
            block=await asyncio.wait_for(reader.read(65536),45)
            if not block:return
            writer.write(block)
            await asyncio.wait_for(writer.drain(),10)
    async def connection(self,reader,writer):
        if len(self.tasks)>=8:
            writer.close();return
        task=asyncio.current_task();self.tasks.add(task);remote=None;pipes=[]
        try:
            upstream,remote=await asyncio.wait_for(asyncio.open_connection(self.host,80,limit=65536),3)
            pipes=[asyncio.create_task(self.pipe(reader,remote)),asyncio.create_task(self.pipe(upstream,writer))]
            done,_=await asyncio.wait(pipes,timeout=180,return_when=asyncio.FIRST_COMPLETED)
            for result in done:result.result()
        except (OSError,TimeoutError,ConnectionError):pass
        finally:
            for item in pipes:item.cancel()
            await asyncio.gather(*pipes,return_exceptions=True)
            for stream in (writer,remote):
                if stream:stream.close()
            self.tasks.discard(task)
    async def __aexit__(self,*_):
        self.server.close();await self.server.wait_closed()
        tasks=list(self.tasks)
        for task in tasks:task.cancel()
        await asyncio.gather(*tasks,return_exceptions=True)

async def trigger_reminders(lab,directory):
    async def wait_for_request():
        while not (directory/'reminder.request').exists():
            await asyncio.sleep(.1)
    await asyncio.wait_for(wait_for_request(),180)
    job_class=r'OCA\Talk\BackgroundJob\Reminder'
    rows=await asyncio.to_thread(lab.occ,'background-job:list','--class',job_class)
    jobs=[parts[1].strip() for row in rows.splitlines()
          if len(parts:=row.split('|'))>=4 and parts[1].strip().isdigit() and parts[2].strip()==job_class]
    lab.require('alpha.reminder-job',len(jobs)==1)
    await asyncio.to_thread(lab.occ,'background-job:execute',jobs[0],'--force-execute')
    (directory/'reminder.done').touch(mode=0o600)

async def exercise(state,manifest,output):
    root=pathlib.Path(__file__).resolve().parents[2]
    source=json.loads(pathlib.Path(manifest).read_text())
    for name,digest in source.items():
        if hashlib.sha256((root/name).read_bytes()).hexdigest()!=digest:raise RuntimeError("Source manifest mismatch")
    # Archive mtimes can predate remote build completion. Force source dirtiness
    # after hash validation so Cargo cannot reuse an earlier source snapshot.
    for name in source:
        if name.endswith(".rs") or name.endswith("Cargo.toml"):os.utime(root/name,None)
    with owned_state(state) as (_,record):
        lab=restore(record)
        room=lab.request('lab_alice','POST','/ocs/v2.php/apps/spreed/api/v4/room',body={'roomType':2,'roomName':'Alpha scoped fixture'},expected=(201,),name='alpha-create-room')
        lab.request('lab_alice','POST','/ocs/v2.php/apps/spreed/api/v4/room/'+room['token']+'/participants',body={'newParticipant':'lab_bob','source':'users'},name='alpha-member-intake')
        listed=lab.request('lab_bob','POST','/ocs/v2.php/apps/spreed/api/v4/room',body={'roomType':2,'roomName':'Disposable discovery fixture'},expected=(201,),name='alpha-create-listed')
        lab.request('lab_bob','PUT','/ocs/v2.php/apps/spreed/api/v4/room/'+listed['token']+'/listable',body={'scope':1},name='alpha-listable')
        lab.request('lab_bob','PUT','/ocs/v2.php/apps/spreed/api/v4/room/'+listed['token']+'/description',body={'description':'Synthetic open conversation'},name='alpha-description')
        directory=pathlib.Path(tempfile.mkdtemp(prefix='nmt-alpha-protocol-',dir='/run'))
        os.chmod(directory,0o700)
        intake=directory/'alice.json'
        intake.write_text(json.dumps({'origin':record['development']['client_url'],'user':'lab_alice','password':lab.passwords['lab_alice'],'room':room}))
        os.chmod(intake,0o600)
        member=directory/'bob.json'
        member.write_text(json.dumps({'origin':record['development']['client_url'],'user':'lab_bob','password':lab.passwords['lab_bob'],'room':room}))
        os.chmod(member,0o600)
        parsed=urllib.parse.urlsplit(record['development']['client_url'])
        host=urllib.parse.urlsplit(record['development']['vm_url']).hostname
        env=os.environ.copy();env['PATH']='/root/.cargo/bin:'+env.get('PATH','');env['NMT_FIXTURE_DIR']=str(directory);env['RUST_BACKTRACE']='0'
        target=pathlib.Path(output);target.mkdir(parents=True,exist_ok=True)
        command=['cargo','test','--locked','-p','talk-adapters','--features','backend-test','--test','alpha_backend','--','--ignored','--nocapture']
        start=time.monotonic()
        try:
            async with Relay(host,parsed.port):
                with (target/'backend.log').open('w') as log:
                    process=await asyncio.create_subprocess_exec(*command,cwd=root,env=env,stdout=log,stderr=asyncio.subprocess.STDOUT)
                    reminder_job=asyncio.create_task(trigger_reminders(lab,directory))
                    try:
                        rc=await asyncio.wait_for(process.wait(),240)
                        if rc==0:await reminder_job
                    except BaseException:
                        if process.returncode is None:process.kill();await process.wait()
                        raise
                    finally:
                        reminder_job.cancel()
                        await asyncio.gather(reminder_job,return_exceptions=True)
            report={'profile':record['development']['profile'],'source':source,'command':command,'exit_code':rc,'seconds':round(time.monotonic()-start,2),'scope':'synthetic released Talk protocol fixture, no GUI/media claim'}
            (target/'result.json').write_text(json.dumps(report,indent=2)+'\n')
            print(json.dumps({'profile':report['profile'],'exit_code':rc,'seconds':report['seconds']}),flush=True)
            return rc
        finally:
            shutil.rmtree(directory)
            lab.request('lab_alice','DELETE','/ocs/v2.php/apps/spreed/api/v4/room/'+room['token'],expected=(200,404),name='alpha-cleanup-room')
            lab.request('lab_bob','DELETE','/ocs/v2.php/apps/spreed/api/v4/room/'+listed['token'],expected=(200,404),name='alpha-cleanup-listed')
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--state',required=True);parser.add_argument('--manifest',required=True);parser.add_argument('--output',required=True)
    args=parser.parse_args();sys.exit(asyncio.run(exercise(args.state,args.manifest,args.output)))
