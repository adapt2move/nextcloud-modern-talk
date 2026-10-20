#!/usr/bin/env python3
"""Owned Linux VM only: isolated private Secret Service, never the user's bus/store.

Native protection remains the OS provider's own. The random fixture unlock secret
exists only in this process/stdin; no password argument, environment value or log.
"""
import argparse
import contextlib
import json
import os
from pathlib import Path
import secrets
import selectors
import shutil
import subprocess
import tempfile
import time

class NativeService:
    def __init__(self):
        self.root=None; self.bus=None; self.daemon=None; self.password=None; self.env=None
    @staticmethod
    def stop(process):
        if process is None:return
        if process.poll() is None:
            process.terminate()
            try:process.wait(3)
            except subprocess.TimeoutExpired:process.kill();process.wait(3)
    def __enter__(self):
        self.root=Path(tempfile.mkdtemp(prefix='nmt-identity-native-',dir='/run'))
        self.root.chmod(0o700)
        try:
            for name in ['runtime','data','control']:(self.root/name).mkdir(mode=0o700)
            config=self.root/'bus.conf'
            config.write_text('<busconfig><type>session</type><listen>unix:path='+str(self.root/'bus')+'</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*" eavesdrop="true"/><allow eavesdrop="true"/><allow own="*"/></policy></busconfig>')
            config.chmod(0o600)
            self.bus=subprocess.Popen(['dbus-daemon','--nofork','--print-address=1','--config-file='+str(config)],stdout=subprocess.PIPE,stderr=subprocess.DEVNULL)
            selector=selectors.DefaultSelector();selector.register(self.bus.stdout,selectors.EVENT_READ)
            if not selector.select(3):raise RuntimeError('private bus startup')
            address=self.bus.stdout.readline(4096).decode().strip();selector.close()
            if not address.startswith('unix:path='+str(self.root/'bus')):raise RuntimeError('private bus identity')
            self.env=os.environ.copy();self.env.update(DBUS_SESSION_BUS_ADDRESS=address,XDG_RUNTIME_DIR=str(self.root/'runtime'),XDG_DATA_HOME=str(self.root/'data'),GNOME_KEYRING_CONTROL=str(self.root/'control'),NMT_PRIVATE_SECRET_SERVICE_TEST='1')
            self.password=secrets.token_urlsafe(32).encode()
            self.restart()
            return self
        except BaseException:
            self.__exit__(None,None,None);raise
    def restart(self):
        self.stop(self.daemon);self.daemon=None
        self.daemon=subprocess.Popen(['gnome-keyring-daemon','--foreground','--unlock','--components=secrets','--control-directory='+str(self.root/'control')],env=self.env,stdin=subprocess.PIPE,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        self.daemon.stdin.write(self.password);self.daemon.stdin.close()
        until=time.monotonic()+8
        while time.monotonic()<until:
            result=subprocess.run(['gdbus','call','--session','--dest','org.freedesktop.secrets','--object-path','/org/freedesktop/secrets','--method','org.freedesktop.Secret.Service.ReadAlias','default'],env=self.env,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,timeout=2)
            if result.returncode==0 and b'/org/freedesktop/secrets/collection/' in result.stdout:return
            if self.daemon.poll() is not None:break
            time.sleep(.1)
        raise RuntimeError('private secret service startup')
    def __exit__(self,*_):
        self.stop(self.daemon);self.stop(self.bus)
        self.password=None
        if self.root:shutil.rmtree(self.root)
        self.daemon=None;self.bus=None

@contextlib.contextmanager
def private_service():
    with NativeService() as service:yield service

def main():
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument('command',nargs=argparse.REMAINDER);args=parser.parse_args()
    command=args.command[1:] if args.command[:1]==['--'] else args.command
    if not command:parser.error('command required')
    result=None
    try:
        with NativeService() as service:
            result=subprocess.run(command,env=service.env,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=180)
            if len(result.stdout)>512*1024:raise RuntimeError('bounded test output')
            print(result.stdout.decode('utf-8',errors='replace'),end='')
        print(json.dumps({'private_secret_service_cleanup':True,'command_exit':result.returncode}))
        return result.returncode
    except Exception as error:
        print(json.dumps({'result':'fail','stage':'private-native-fixture','error_kind':type(error).__name__}))
        return 1
if __name__=='__main__':raise SystemExit(main())
