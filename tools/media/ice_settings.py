#!/usr/bin/env python3
"""Qualify UDP STUN and forced TURN using settings issued by the owned lab.

Settings/credentials are held only in memory and one private temporary fixture.
The report contains the native probe's redacted candidate/selected-pair counts.
"""
import argparse
import asyncio
import base64
import json
import os
from pathlib import Path
import ssl
import subprocess
import sys
import tempfile
from urllib.parse import urlparse, urlencode
import urllib.request


def fetch_settings(origin, password, token, user='lab_alice'):
    auth=base64.b64encode((user+':'+password).encode()).decode()
    req=urllib.request.Request(origin+'/ocs/v2.php/apps/spreed/api/v3/signaling/settings?'+urlencode({'token':token}),headers={'Authorization':'Basic '+auth,'OCS-APIRequest':'true','Accept':'application/json'})
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self,*_):return None
    with urllib.request.build_opener(urllib.request.ProxyHandler({}),NoRedirect()).open(req,timeout=3) as response:
        raw=response.read(65537)
        if len(raw)>65536:raise RuntimeError('settings response bound')
        body=json.loads(raw)['ocs']
        if body['meta']['status']!='ok':raise RuntimeError('settings refused')
        return body['data']


async def run(a):
    from talk_browser import LoopbackForward, private_json

    m=private_json(a.manifest);c=private_json(m['credentials_file']);fixtures=private_json(m['fixtures_file']);origin=urlparse(m['client_url']);target=urlparse(m['vm_url'])
    tls=None
    if origin.scheme=='https':
        tls=ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER);root=Path(m['media_tls']);tls.load_cert_chain(root/'cert.pem',root/'key.pem')
    async with LoopbackForward(target.hostname,origin.port,ssl_context=tls):
        settings=await asyncio.to_thread(fetch_settings,m['client_url'],c['users']['lab_alice'],fixtures['group_token'])
        with tempfile.TemporaryDirectory(prefix='nmt-ice-settings-',dir='/run') as directory:
            path=Path(directory)/'fixture.json'
            servers=settings['turnservers'] if a.expect=='relay' else settings['stunservers']
            path.write_text(json.dumps({'policy':'relay' if a.expect=='relay' else 'all','servers':servers}));path.chmod(0o600)
            result=await asyncio.create_subprocess_exec(sys.executable,str(Path(__file__).with_name('ice_native.py')),'--executable',str(a.executable),'--fixture',str(path),'--expect',a.expect,'--output',str(a.output))
            code=await result.wait()
            report=json.loads(a.output.read_text());report['settings_source']='Talk signaling settings';report['settings_credentials_persisted']=False;report['advertised_servers']=len(servers);a.output.write_text(json.dumps(report,indent=2)+'\n')
            return code


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--manifest',type=Path,required=True);p.add_argument('--executable',type=Path,required=True);p.add_argument('--expect',choices=['srflx','relay'],required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args();os.umask(0o077);raise SystemExit(asyncio.run(run(a)))
if __name__=='__main__':main()
