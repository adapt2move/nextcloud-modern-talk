#!/usr/bin/env python3
"""Malformed IPC tests against real helper/supervisor; no mocks or wire logging."""
import argparse
import json
from pathlib import Path
import struct
import subprocess
import time
from interop import Peer, HEADER, sha


def check(driver,helper,kind):
    peer=Peer(driver,helper,intent=0)
    deadline=time.monotonic()+3
    while peer.ready_ms is None and time.monotonic()<deadline:
        peer.read();time.sleep(.005)
    if peer.ready_ms is None:
        raise RuntimeError("native ready absent")
    peer.sample()
    if kind=="record-too-large":
        peer.child.stdin.write(struct.pack(">I",8193));peer.child.stdin.flush()
    elif kind=="sdp-too-large":
        peer.send(3,struct.pack(">IIB",65537,0,1)+b"v=0\r\n")
    elif kind=="wrong-generation":
        peer.scope=bytes([peer.scope[0]^1])+peer.scope[1:]
        peer.send(6)
    elif kind=="stale-serial":
        peer.serial=0;peer.send(6)
    elif kind=="duplicate-create":
        peer.send(1,b"\0")
    start=time.monotonic()
    peer.child.wait(timeout=1)
    elapsed=(time.monotonic()-start)*1000
    # Wrong-scope injection only affects input; restore expected event identity.
    if kind=="wrong-generation":
        peer.scope=bytes([peer.scope[0]^1])+peer.scope[1:]
    stop=peer.stop()
    return {"case":kind,"exit_ms":elapsed,"driver_exit":peer.child.returncode,"cleanup":stop,"pass":elapsed<250 and stop["helper_gone_ms"] is not None}


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument("--driver",type=Path,required=True);p.add_argument("--helper",type=Path,required=True);p.add_argument("--output",type=Path,required=True)
    args=p.parse_args()
    report={"schema":1,"driver_sha256":sha(args.driver),"helper_sha256":sha(args.helper),"harness_sha256":sha(__file__),"cases":[]}
    for kind in ["record-too-large","sdp-too-large","wrong-generation","stale-serial","duplicate-create"]:
        report["cases"].append(check(args.driver,args.helper,kind))
    # The helper itself rejects an unchecked oversized prefix before reading or
    # allocating a body, independently of the Rust framing validator.
    start=time.monotonic()
    child=subprocess.run([str(args.helper),"--diagnostic-synthetic","/nonexistent"],input=struct.pack(">I",0xffffffff),stdout=subprocess.PIPE,stderr=subprocess.PIPE,timeout=1)
    report["cases"].append({"case":"native-oversize-before-body","exit":child.returncode,"elapsed_ms":(time.monotonic()-start)*1000,"pass":child.returncode==64 and not child.stdout and not child.stderr})
    report["result"]="pass" if all(c["pass"] for c in report["cases"]) else "fail"
    args.output.write_text(json.dumps(report,indent=2)+"\n")
    print(json.dumps({"result":report["result"]}))
    raise SystemExit(0 if report["result"]=="pass" else 1)


if __name__=="__main__":main()
