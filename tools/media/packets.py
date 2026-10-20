#!/usr/bin/env python3
"""Bounded temporary UDP capture; exports counts/times only, never packet bytes."""
from pathlib import Path
import signal
import struct
import subprocess
import tempfile
import time


def process_observation(pid):
    inodes = set()
    opened_devices = 0
    try:
        for path in Path(f"/proc/{pid}/fd").iterdir():
            try:
                target = str(path.readlink())
                if target.startswith("socket:["):
                    inodes.add(target[8:-1])
                if target.startswith(("/dev/video", "/dev/snd/")):
                    opened_devices += 1
            except (FileNotFoundError, ProcessLookupError):
                pass
    except (FileNotFoundError, ProcessLookupError):
        pass
    ports = set()
    for name in ("udp", "udp6"):
        for row in Path("/proc/net/" + name).read_text().splitlines()[1:]:
            fields = row.split()
            if fields[9] in inodes:
                ports.add(int(fields[1].split(":")[1], 16))
    return ports, opened_devices


def running(pid):
    try:
        return Path(f"/proc/{pid}/stat").read_text().split(")", 1)[1].strip().split()[0] != "Z"
    except (FileNotFoundError, ProcessLookupError):
        return False


class Capture:
    def __enter__(self):
        self.directory = tempfile.TemporaryDirectory(prefix="nmt-media-packets-",dir="/run")
        self.path = Path(self.directory.name) / "udp.pcap"
        self.child = subprocess.Popen(["tcpdump","--immediate-mode","-i","any","-Q","inout","-n","-s","96","-c","20000","-U","-w",str(self.path),"udp"],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        time.sleep(.15)
        if self.child.poll() is not None:
            raise RuntimeError("bounded packet observer unavailable")
        return self

    def finish(self, observations):
        self.child.send_signal(signal.SIGINT)
        self.child.wait(timeout=3)
        summaries=[{"observed_udp_packets":0,"packets_after_revoke_request":0,"last_packet_minus_revoke_ms":None} for _ in observations]
        with self.path.open("rb") as stream:
            header=stream.read(24)
            if len(header)!=24 or header[:4] not in (b"\xd4\xc3\xb2\xa1",b"\xa1\xb2\xc3\xd4"):
                raise RuntimeError("unsupported packet capture header")
            endian="<" if header[:4]==b"\xd4\xc3\xb2\xa1" else ">"
            link=struct.unpack_from(endian+"I",header,20)[0]
            offset={276:20,113:16,1:14}.get(link)
            if offset is None:
                raise RuntimeError("unsupported packet link profile")
            for _ in range(20000):
                record=stream.read(16)
                if not record:
                    break
                if len(record)!=16:
                    raise RuntimeError("truncated packet record")
                sec,micro,size,_=struct.unpack(endian+"4I",record)
                if size>96:
                    raise RuntimeError("packet observer bound")
                packet=stream.read(size)
                if len(packet)!=size or len(packet)<offset+20:
                    continue
                version=packet[offset]>>4
                if version==4 and packet[offset+9]==17:
                    udp=offset+(packet[offset]&15)*4
                elif version==6 and len(packet)>=offset+44 and packet[offset+6]==17:
                    udp=offset+40
                else:
                    continue
                if len(packet)<udp+4:
                    continue
                source=struct.unpack_from(">H",packet,udp)[0]
                for result,(ports,revoke) in zip(summaries,observations):
                    if source not in ports:
                        continue
                    delta=(sec+micro/1000000-revoke)*1000
                    result["observed_udp_packets"]+=1
                    result["packets_after_revoke_request"]+=int(delta>=0)
                    result["last_packet_minus_revoke_ms"]=delta
        return {"method":"tcpdump immediate any/inout UDP (includes loopback delivery), 96-byte snaplen, max20000 packets, local wall clock", "peers":summaries,
                "uncertainty":"timestamp rounding, scheduler/poll latency and already queued kernel packets; only this same-VM profile", "raw_capture":"deleted; no packet bytes exported"}

    def __exit__(self,*_):
        if self.child.poll() is None:
            self.child.kill()
            self.child.wait(timeout=3)
        self.directory.cleanup()
