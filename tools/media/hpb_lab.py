#!/usr/bin/env python3
"""Opt-in HPB/Janus/UDP TURN adjunct on the owning disposable media VM.

Requires a running tests/lab/dev.py lab, Janus 1.3.0 installed at
/opt/call-media-janus, coturn, and the exact signaling image below. Private
configuration and process logs stay in the lab runtime and die with its VM.
Use --stop before tests/lab/dev.py stop. No host public ports are opened.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import signal
import subprocess
import sys
import time
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'tests/lab'))
from dev import owned_state, restore, check_ownership

IMAGE = 'strukturag/nextcloud-spreed-signaling@sha256:b306eae9d1e62c4b2eaf10a6683dfe1fe76b833bd0f68d1606861e33aff5bca1'
# Bounded, disposable container-loopback bridge. This keeps server-to-server
# hooks on the same localhost URL the clients receive, without weakening URL
# validation or publishing Docker ports.
PROXY = r'''<?php
$s=stream_socket_server('tcp://127.0.0.1:8080'); if(!$s)exit(2);
stream_set_blocking($s,false);$pairs=[];$born=[];
while(true){$read=[$s];foreach($pairs as $key=>$value)$read[]=$value[0];$write=null;$except=null;
if(stream_select($read,$write,$except,1)===false)break;
foreach($read as $r){if($r===$s){$a=stream_socket_accept($s,0);if(!$a)continue;
if(count($pairs)>=64){fclose($a);continue;}$b=stream_socket_client('tcp://GATEWAY:8080',$eno,$err,2);if(!$b){fclose($a);continue;}
stream_set_timeout($a,2);stream_set_timeout($b,2);$pairs[(int)$a]=[$a,$b];$pairs[(int)$b]=[$b,$a];$born[(int)$a]=$born[(int)$b]=time();continue;}
$k=(int)$r;$data=fread($r,65536);$other=$pairs[$k][1];
if($data===false||$data===''||time()-$born[$k]>180){unset($pairs[$k],$pairs[(int)$other],$born[$k],$born[(int)$other]);fclose($r);fclose($other);continue;}
for($i=0;$i<strlen($data);){$n=fwrite($other,substr($data,$i));if(!$n)break;$i+=$n;}}
}'''


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--state', type=Path, required=True)
    p.add_argument('--serve', action='store_true', help='Keep the owned native services alive until stopped')
    p.add_argument('--stop', action='store_true')
    p.add_argument('--restart-signaling', action='store_true')
    args = p.parse_args()
    os.umask(0o077)
    with owned_state(args.state) as (_, state):
        lab = restore(state)
        check_ownership(lab)
        client_url = json.loads((lab.runtime/'manifest.json').read_text())['client_url']
        root = lab.runtime / 'media-backend'
        record = root / 'owned.json'
        if args.restart_signaling:
            data=json.loads(record.read_text());subprocess.run(['docker','rm','-f',data['container']],check=True,stdout=subprocess.DEVNULL)
            launch_signaling(root,data['container'],lab.runtime/'media-tls' if client_url.startswith('https:') else None)
            print(json.dumps({'result':'signaling restarted'}));return
        if args.stop:
            data = json.loads(record.read_text())
            subprocess.run(['docker', 'rm', '-f', data['container']], check=True, stdout=subprocess.DEVNULL)
            for pid in data['pids']:
                try: os.kill(pid, signal.SIGTERM)
                except ProcessLookupError: pass
            subprocess.run(['docker','exec',lab.container,'pkill','-f','/tmp/nmt-hpb-loopback.php'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            configure(lab, {'signaling_servers':json.dumps({'servers':[], 'secret':''}), 'stun_servers':'[]', 'turn_servers':'[]'})
            print(json.dumps({'result':'stopped'})); return
        if not args.serve:raise RuntimeError('--serve is required; run this tool as an owned detached VM job')
        root.mkdir(mode=0o700)
        network = json.loads(lab.docker('network','inspect',state['networks'][0]))[0]
        gateway = network['IPAM']['Config'][0]['Gateway']
        secret, turn_secret = secrets.token_hex(32), secrets.token_hex(32)
        (root/'server.conf').write_text(f'''[http]
listen = {gateway}:8080
[sessions]
hashkey = {secrets.token_hex(32)}
blockkey = {secrets.token_hex(16)}
[clients]
internalsecret = {secrets.token_hex(32)}
[backend]
backends = lab
[lab]
urls = {client_url}
secret = {secret}
[nats]
url = nats://loopback
[mcu]
type = janus
url = ws://127.0.0.1:8188
maxscreenbitrate = 2097152
''')
        conf = root/'janus';conf.mkdir()
        (conf/'janus.jcfg').write_text(f'''general: {{
plugins_folder = "/opt/call-media-janus/lib/janus/plugins"
transports_folder = "/opt/call-media-janus/lib/janus/transports"
events_folder = "/opt/call-media-janus/lib/janus/events"
}}
media: {{ rtp_port_range = "20000-20200" }}
nat: {{ ice_enforce_list = "{gateway}" }}
transports: {{ disable = "libjanus_http.so,libjanus_pfunix.so" }}
''')
        (conf/'janus.transport.websockets.jcfg').write_text('general: { ws = true ws_port = 8188 ws_ip = "127.0.0.1" wss = false }\nadmin: { admin_ws = false admin_wss = false }\n')
        (conf/'janus.plugin.videoroom.jcfg').write_text('general: { }\n')
        (root/'turn.conf').write_text(f'''listening-ip={gateway}
relay-ip={gateway}
listening-port=3478
min-port=20300
max-port=20400
realm=nmt-disposable
use-auth-secret
static-auth-secret={turn_secret}
allow-loopback-peers
no-tcp
no-tls
no-dtls
no-cli
no-multicast-peers
''')
        name = 'nmt-media-hpb-' + state['run_id']
        data = {'container':name,'pids':[], 'gateway':gateway}
        record.write_text(json.dumps(data))
        processes=[]
        for label, argv in [('janus',['/opt/call-media-janus/bin/janus','-F',str(conf)]), ('turn',['turnserver','-c',str(root/'turn.conf')]), ('stun-proxy',[sys.executable,str(Path(__file__).with_name('stun_proxy.py')),'--host',gateway])]:
            with (root/(label+'.log')).open('wb') as output:
                proc = subprocess.Popen(argv,stdout=output,stderr=subprocess.STDOUT,start_new_session=True)
            processes.append(proc)
            data['pids'].append(proc.pid);record.write_text(json.dumps(data))
        version_deadline=time.monotonic()+5
        while time.monotonic()<version_deadline:
            if 'Janus version: 1300 (1.3.0)' in (root/'janus.log').read_text(errors='replace'):break
            if processes[0].poll() is not None:raise RuntimeError('Janus exited before readiness')
            time.sleep(.05)
        else:
            for proc in processes:proc.terminate()
            raise RuntimeError('Janus 1.3.0 runtime identity not observed')
        launch_signaling(root, name, lab.runtime/'media-tls' if client_url.startswith('https:') else None)
        lab.command('docker','exec','-i',lab.container,'sh','-c','cat > /tmp/nmt-hpb-loopback.php',stdin=PROXY.replace('GATEWAY',gateway))
        lab.docker('exec','-d',lab.container,'php','/tmp/nmt-hpb-loopback.php')
        configure(lab, {'signaling_servers':json.dumps({'servers':[{'server':('https://'+gateway+':8081') if client_url.startswith('https:') else 'http://127.0.0.1:8080','verify':True}], 'secret':secret}), 'stun_servers':json.dumps([gateway+':3479']), 'turn_servers':json.dumps([{'server':gateway+':3478','secret':turn_secret,'protocols':'udp'}])})
        print(json.dumps({'result':'started','signaling_image':IMAGE,'janus':'1.3.0','hpb_target':gateway,'source_sha256':hashlib.sha256(Path(__file__).read_bytes()).hexdigest()}),flush=True)

    def stopped(*_):raise SystemExit(0)
    signal.signal(signal.SIGTERM,stopped)
    try:
        while all(proc.poll() is None for proc in processes):time.sleep(1)
    finally:
        for proc in processes:
            if proc.poll() is None:proc.terminate()
        for proc in processes:
            try:proc.wait(timeout=3)
            except subprocess.TimeoutExpired:proc.kill();proc.wait()


def launch_signaling(root, name, tls):
    trust=['--mount',f'type=bind,src={tls}/ca.pem,dst=/lab-ca.pem,readonly','--env','SSL_CERT_FILE=/lab-ca.pem'] if tls else []
    subprocess.run(['docker','run','-d','--name',name,'--network','host','--read-only','--cap-drop','ALL','--security-opt','no-new-privileges','--memory','256m','--pids-limit','128','--mount',f'type=bind,src={root}/server.conf,dst=/server.conf,readonly',*trust,'--entrypoint','/usr/bin/nextcloud-spreed-signaling',IMAGE,'-config','/server.conf'],check=True,stdout=subprocess.DEVNULL)


def configure(lab, values):
    # Config values containing credentials enter only via stdin, never argv/env.
    code = "require '/var/www/html/lib/base.php'; foreach(json_decode(stream_get_contents(STDIN),true) as $k=>$v) { \\OC::$server->get(\\OCP\\IConfig::class)->setAppValue('spreed',$k,$v); }"
    lab.command('docker','exec','-i','--user','www-data',lab.container,'php','-r',code,stdin=json.dumps(values))

if __name__ == '__main__': main()
