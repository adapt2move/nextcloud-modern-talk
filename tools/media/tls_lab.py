#!/usr/bin/env python3
"""Create a private test CA for the owned disposable VM and its NextCloud container.

Never use on a workstation or production server. The VM must be destroyed after
qualification. The CA key remains in the private /run lab runtime, not evidence.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[2]/'tests/lab'))
from dev import owned_state, restore, check_ownership


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--state',type=Path,required=True);a=p.parse_args();os.umask(0o077)
    with owned_state(a.state) as (state_path,state):
        lab=restore(state);check_ownership(lab)
        gateway=json.loads(lab.docker('network','inspect',state['networks'][0]))[0]['IPAM']['Config'][0]['Gateway']
        root=lab.runtime/'media-tls';root.mkdir(mode=0o700,exist_ok=True)
        config=root/'ca.cnf';config.write_text('[req]\ndistinguished_name=dn\nx509_extensions=ext\nprompt=no\n[dn]\nCN=NMT disposable media CA\n[ext]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n')
        subprocess.run(['openssl','req','-x509','-newkey','rsa:2048','-nodes','-days','1','-config',str(config),'-keyout',str(root/'ca-key.pem'),'-out',str(root/'ca.pem')],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        leaf=root/'leaf.cnf';leaf.write_text('[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=NMT disposable media server\n')
        ext=root/'leaf.ext';ext.write_text('basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=IP:127.0.0.1,IP:'+gateway+'\n')
        subprocess.run(['openssl','req','-new','-newkey','rsa:2048','-nodes','-config',str(leaf),'-keyout',str(root/'key.pem'),'-out',str(root/'server.csr')],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['openssl','x509','-req','-in',str(root/'server.csr'),'-CA',str(root/'ca.pem'),'-CAkey',str(root/'ca-key.pem'),'-CAcreateserial','-days','1','-extfile',str(ext),'-out',str(root/'cert.pem')],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        subprocess.run(['cp',str(root/'ca.pem'),'/usr/local/share/ca-certificates/nmt-disposable-media.crt'],check=True)
        subprocess.run(['update-ca-certificates'],check=True,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        lab.docker('cp',str(root/'ca.pem'),lab.container+':/usr/local/share/ca-certificates/nmt-disposable-media.crt')
        lab.docker('exec',lab.container,'chmod','644','/usr/local/share/ca-certificates/nmt-disposable-media.crt')
        lab.docker('exec',lab.container,'update-ca-certificates')
        lab.occ('security:certificates:import','/usr/local/share/ca-certificates/nmt-disposable-media.crt')
        # Browser trust uses this disposable VM's NSS store, not a disabled TLS
        # verifier. The application uses ordinary system roots too.
        for db in [Path('/root/.pki/nssdb'), Path('/root/.local/share/pki/nssdb')]:
            db.mkdir(parents=True,exist_ok=True)
            if not (db/'cert9.db').exists():subprocess.run(['certutil','-N','--empty-password','-d','sql:'+str(db)],check=True)
            subprocess.run(['certutil','-D','-d','sql:'+str(db),'-n','nmt-disposable-media'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
            subprocess.run(['certutil','-A','-d','sql:'+str(db),'-n','nmt-disposable-media','-t','C,,','-i',str(root/'ca.pem')],check=True)
        manifest_path=lab.runtime/'manifest.json';manifest=json.loads(manifest_path.read_text());manifest['client_url']=state['development']['client_url'].replace('http:','https:',1);manifest['media_tls']=str(root);manifest_path.write_text(json.dumps(manifest))
        lab.occ('config:system:set','overwriteprotocol','--value=https')
        lab.occ('config:system:set','overwrite.cli.url','--value='+manifest['client_url'])
        print(json.dumps({'result':'configured','hpb_target':gateway,'ca':'disposable VM only; removed with VM'}))

if __name__=='__main__':main()
