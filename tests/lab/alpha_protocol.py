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
        directory=pathlib.Path(tempfile.mkdtemp(prefix='nmt-alpha-protocol-',dir='/run'))
        os.chmod(directory,0o700)
        intake=directory/'alice.json'
        intake.write_text(json.dumps({'origin':record['development']['client_url'],'user':'lab_alice','password':lab.passwords['lab_alice'],'room':room}))
        os.chmod(intake,0o600)
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
                    try:rc=await asyncio.wait_for(process.wait(),240)
                    except BaseException:
                        process.kill();await process.wait();raise
            report={'profile':record['development']['profile'],'source':source,'command':command,'exit_code':rc,'seconds':round(time.monotonic()-start,2),'scope':'synthetic released Talk protocol fixture, no GUI/media claim'}
            (target/'result.json').write_text(json.dumps(report,indent=2)+'\n')
            print(json.dumps({'profile':report['profile'],'exit_code':rc,'seconds':report['seconds']}),flush=True)
            return rc
        finally:
            shutil.rmtree(directory)
            lab.request('lab_alice','DELETE','/ocs/v2.php/apps/spreed/api/v4/room/'+room['token'],expected=(200,404),name='alpha-cleanup-room')
if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--state',required=True);parser.add_argument('--manifest',required=True);parser.add_argument('--output',required=True)
    args=parser.parse_args();sys.exit(asyncio.run(exercise(args.state,args.manifest,args.output)))
