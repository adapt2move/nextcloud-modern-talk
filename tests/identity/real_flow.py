#!/usr/bin/env python3
"""Owned VM only: actual browser grant and Rust identity lifecycle, allowlisted evidence."""
import argparse
import contextlib
import asyncio
import hashlib
import importlib.metadata
import ipaddress
import json
import os
from pathlib import Path
import shutil
import sys
import time
from urllib.parse import urlsplit
sys.dont_write_bytecode = True
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'lab'))
from dev import owned_state, private_json, restore
from playwright.async_api import async_playwright
from native_service import NativeService

def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()

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

class Driver:
    def __init__(self,binary,intake,report):self.binary,self.intake,self.report=binary,intake,report;self.process=None;self.env=None
    async def start(self):
        self.process=await asyncio.create_subprocess_exec(str(self.binary),str(self.intake),stdin=asyncio.subprocess.PIPE,stdout=asyncio.subprocess.PIPE,stderr=asyncio.subprocess.DEVNULL,limit=8192,env=self.env)
        if await self.reply()!={'ready':True}:raise RuntimeError('driver startup')
    async def reply(self,timeout=12):
        value=await asyncio.wait_for(self.process.stdout.readline(),timeout)
        if not value or len(value)>4096:raise RuntimeError('driver bounded response')
        result=json.loads(value)
        if set(result)-{'ready','phase','storage','family','accounts','suppressed','suppression_unconfirmed','error','fatal','rooms','keys','preferences','favorite','archive','tags','send'}:raise RuntimeError('driver output schema')
        return result
    async def call(self,command):
        self.process.stdin.write((command+'\n').encode('ascii'));await self.process.stdin.drain();reply=await self.reply(60 if command=='features' else 12)
        self.report['last_command']=command;self.report['last_reply']=reply
        return reply
    async def stop(self):
        if not self.process:return
        if self.process.returncode is None:
            try:
                self.process.stdin.write(b'quit\n');await self.process.stdin.drain()
                await asyncio.wait_for(self.process.wait(),10)
            except (BrokenPipeError,ConnectionResetError,TimeoutError):
                self.process.kill();await self.process.wait()
        code=self.process.returncode;self.process=None
        if code:raise RuntimeError('driver exit')

async def grant(page,url,password,report,login_name):
    await page.goto(url,wait_until='domcontentloaded',timeout=20000)
    actions=[]
    for _ in range(30):
        if await page.locator('#password').count() and await page.locator('#password').is_visible():
            if await page.locator('#user').count():await page.locator('#user').fill(login_name)
            await page.locator('#password').fill(password)
            await page.locator('button[type=submit],input[type=submit]').first.click()
            actions.append('submit-synthetic-login')
        elif await page.get_by_role('button',name='Grant access',exact=True).count():
            await page.get_by_role('button',name='Grant access',exact=True).click();actions.append('grant-access')
        elif await page.get_by_role('button',name='Log in',exact=True).count():
            await page.get_by_role('button',name='Log in',exact=True).click();actions.append('open-login')
        elif await page.get_by_role('link',name='Log in',exact=True).count():
            await page.get_by_role('link',name='Log in',exact=True).click();actions.append('open-login')
        elif await page.get_by_text('Account connected',exact=False).count():
            report['browser_actions'].append(actions);return
        await asyncio.sleep(0.3)
    report['browser_actions'].append(actions)
    # No raw body, locator values, cookies, URL, screenshot or exception string.
    raise RuntimeError('browser grant not completed')

async def rejection_fixture(origin):
    # No Authorization/Cookie is sent. Keep only fixed enum/numeric response shape,
    # never headers, response values, session IDs or an error message.
    import http.client
    def observe():
        connection=http.client.HTTPConnection(origin.hostname,origin.port,timeout=3)
        try:
            connection.request('GET',origin.path.rstrip('/')+'/ocs/v2.php/cloud/user?format=json',headers={'Accept':'application/json','OCS-APIRequest':'true','Connection':'close'})
            response=connection.getresponse();body=response.read(256*1024+1)
            if len(body)>256*1024:raise RuntimeError('bounded fixture response')
            value=json.loads(body);ocs=value.get('ocs',{});meta=ocs.get('meta',{})
            result={'http_status':response.status,'meta_status':'failure' if meta.get('status')=='failure' else 'other',
                    'ocs_code':997 if type(meta.get('statuscode')) is int and meta['statuscode']==997 else None,
                    'message_is_string':isinstance(meta.get('message'),str),
                    'data_container':isinstance(ocs.get('data'),(dict,list)),
                    'scope':'same-origin unauthenticated account-probe fixture; no credential intake'}
            if result['http_status']!=401 or result['ocs_code']!=997 or result['meta_status']!='failure' or not result['message_is_string'] or not result['data_container']:
                raise RuntimeError('unqualified fixture rejection')
            return result
        finally:connection.close()
    return await asyncio.to_thread(observe)

async def run(args,report):
    if importlib.metadata.version('playwright')!='1.64.0':raise RuntimeError('browser version')
    build=json.loads(args.build_record.read_text())
    if build['binary_sha256']!=digest(args.binary):raise RuntimeError('binary provenance mismatch')
    root=Path(__file__).resolve().parents[2]
    for name,expected in build['files'].items():
        relative=Path(name)
        if relative.is_absolute() or '..' in relative.parts or digest(root/relative)!=expected:
            raise RuntimeError('source provenance mismatch')
    report['build_record_sha256']=digest(args.build_record)
    report['source_archive_sha256']=build['source_archive_sha256']
    with owned_state(args.state) as (_,state):
        lab=restore(state);manifest=private_json(lab.runtime/'manifest.json');credentials=private_json(lab.runtime/'credentials.json')
        source=urlsplit(manifest['vm_url']);origin=urlsplit(manifest['client_url'])
        if origin.scheme!='http' or origin.hostname!='127.0.0.1':raise RuntimeError('explicit loopback required')
        report.update(browser_login_kind='email' if args.email else 'uid',profile=manifest['profile'],nextcloud=manifest['nextcloud'],talk=manifest['talk'],lab_sources=manifest['source_sha256'],binary_sha256=digest(args.binary),driver_source_sha256=digest(Path(__file__).with_name('driver.rs')),harness_sha256=digest(__file__),browser_package='1.64.0',browser_actions=[],observations=[])
        login_name='lab_alice'
        if args.email:
            login_name='Synthetic.Identity@Example.invalid'
            lab.occ('user:setting','lab_alice','settings','email',login_name)
        private=lab.runtime/'identity-i2';private.mkdir(mode=0o700)
        intake=private/'intake.json';intake.write_text(json.dumps({'origin':manifest['client_url'],'directory':str(private/'store')}));intake.chmod(0o600)
        driver=Driver(args.binary,intake,report)
        services=contextlib.ExitStack();native=None
        try:
            if args.native:
                native=services.enter_context(NativeService());driver.env=native.env
            async with Relay(source.hostname,origin.port),async_playwright() as playwright:
                browser=await playwright.chromium.launch(headless=True,args=['--no-sandbox','--disable-dev-shm-usage'])
                report['browser_version']=browser.version
                context=await browser.new_context(locale='en-US')
                page=await context.new_page()
                report['authoritative_rejection_fixture']=await rejection_fixture(origin)
                await driver.start()
                for cycle in (['logout'] if args.account_path else ['logout','auth-loss']):
                    report['stage']=cycle+'-begin'
                    reply=await driver.call('begin');assert reply=={'phase':'begun'}
                    reply=await driver.call('poll');assert reply=={'phase':'pending'}
                    url=(private/'store'/'browser.login').read_text()
                    report['stage']=cycle+'-browser-grant'
                    await grant(page,url,credentials['users']['lab_alice'],report,login_name)
                    for _ in range(15):
                        await asyncio.sleep(2)
                        reply=await driver.call('poll')
                        if reply.get('phase')!='pending':break
                    assert reply=={'phase':'authorized'}
                    report['observations'].append({'cycle':cycle,'actual_browser_grant':True,'canonical_app_validation':True})
                    if cycle=='logout' and not args.native:
                        report['stage']='known-native-nonwrite'
                        reply=await driver.call('save-native')
                        assert 'error' in reply and 'Unavailable' in reply['error']
                        report['observations'].append({'native':reply})
                    storage='native' if args.native else 'file'
                    report['stage']=cycle+'-'+storage+'-store'
                    reply=await driver.call('save-'+storage);assert reply.get('phase')=='active' and reply.get('storage')==storage
                    assert reply['family']==('Nextcloud34' if manifest['profile']=='r34' else 'Nextcloud35')
                    assert await driver.call('probe')=={'phase':'authenticated'}
                    if args.account_path:
                        report['stage']='facade-rooms-preferences-tags-send'
                        reply=await driver.call('features')
                        assert reply.get('phase')=='features' and reply['rooms']>0 and reply['keys']==reply['rooms']
                        assert all(reply.get(key) is True for key in ['preferences','favorite','archive','tags','send'])
                        report['observations'].append({'facade_account_path':reply})
                    if args.email:
                        import sqlite3
                        with sqlite3.connect('file:'+str(private/'store'/'restoration.sqlite')+'?mode=ro',uri=True) as db:
                            row=db.execute('SELECT a.user,o.login_name FROM accounts a JOIN operations o ON o.id=a.eligible').fetchone()
                            report['observations'].append({'cycle':cycle,'grant_login_differs_from_canonical':row[0]!=row[1]})
                    await driver.stop()
                    if args.native:
                        native.restart()
                        report['observations'].append({'cycle':cycle,'native_service_restarted':True})
                    elif args.native_return and native is None:
                        native=services.enter_context(NativeService());driver.env=native.env
                        report['observations'].append({'cycle':cycle,'native_service_returned_before_restore':True})
                    await driver.start()
                    report['stage']=cycle+'-restart-restore'
                    reply=await driver.call('restore');assert reply.get('phase')=='active' and reply.get('storage')==storage
                    assert await driver.call('probe')=={'phase':'authenticated'}
                    report['observations'].append({'cycle':cycle,'stored_backend':storage,'new_process_restore':True,'app_probe':True})
                    if cycle=='logout':
                        report['stage']='logout-suppression'
                        await driver.call('logout')
                    else:
                        report['stage']='confirmed-account-loss'
                        # Private fixture lifecycle only. No secret in occ argv/output/report.
                        lab.occ('user:disable','lab_alice')
                        report['observations'].append({'loss_trigger':'disable-synthetic-account'})
                        reply=await driver.call('probe')
                        assert reply=={'error':'Request(ReauthenticationRequired)'}
                    for _ in range(30):
                        reply=await driver.call('state')
                        if reply.get('suppressed'):break
                        await asyncio.sleep(0.1)
                    assert reply.get('suppressed') and not reply.get('suppression_unconfirmed')
                    await driver.stop();await driver.start()
                    assert await driver.call('restore')=={'error':'RecoveryRequired'}
                    report['observations'].append({'cycle':cycle,'durable_suppression':True,'restart_restore_refused':True})
                report['stage']='exact-stored-cleanup'
                assert await driver.call('retire-all')=={'phase':'maintained'}
                assert await driver.call('maintain')=={'phase':'maintained'}
                assert await driver.call('forget')=={'phase':'forgotten'}
                report['observations'].append({'explicit_retirement_cleanup_and_forget':True})
                await driver.stop();await context.close();await browser.close()
                report['result']='pass'
        finally:
            try:await driver.stop()
            finally:services.close()
            report['private_native_service_cleanup']=True
            shutil.rmtree(private)
            report['private_identity_files_removed']=not private.exists()

async def main():
    os.umask(0o077)
    parser=argparse.ArgumentParser(description=__doc__)
    storage=parser.add_mutually_exclusive_group();storage.add_argument('--native',action='store_true');storage.add_argument('--native-return',action='store_true')
    parser.add_argument('--account-path',action='store_true',help='One facade login/rooms/settings/tag/send/restore/logout path; no extra auth-loss cycle')
    parser.add_argument('--build-record',required=True,type=Path);parser.add_argument('--email',action='store_true');parser.add_argument('--state',required=True);parser.add_argument('--binary',required=True,type=Path);parser.add_argument('--output',required=True,type=Path)
    args=parser.parse_args();report={'schema':1,'scope':'identity author real Login Flow, isolated HTTP loopback, no native platform/product approval','result':'fail','stage':'startup'}
    started=time.monotonic()
    try:await asyncio.wait_for(run(args,report),300)
    except Exception as error:report['error_category']=type(error).__name__
    report['elapsed_seconds']=round(time.monotonic()-started,3)
    args.output.write_text(json.dumps(report,indent=2)+'\n');args.output.chmod(0o600)
    print(json.dumps({'result':report['result'],'stage':report['stage']}))
    return 0 if report['result']=='pass' else 1
if __name__=='__main__':raise SystemExit(asyncio.run(main()))
