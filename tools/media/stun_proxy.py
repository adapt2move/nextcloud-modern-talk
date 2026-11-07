#!/usr/bin/env python3
"""Bounded UDP Binding-only NAT fixture; never a production STUN/TURN service."""
import argparse
import asyncio
import time


class Upstream(asyncio.DatagramProtocol):
    def __init__(self, owner, peer):self.owner,self.peer=owner,peer
    def connection_made(self, transport):self.transport=transport
    def datagram_received(self,data,_):
        if len(data)<=2048:self.owner.transport.sendto(data,self.peer)


class Proxy(asyncio.DatagramProtocol):
    def __init__(self,host):self.host=host;self.peers={};self.pending=set()
    def connection_made(self,transport):self.transport=transport
    def datagram_received(self,data,peer):
        if not 20<=len(data)<=2048 or data[:2]!=b'\x00\x01' or data[4:8]!=b'\x21\x12\xa4\x42':return
        if int.from_bytes(data[2:4],'big')+20!=len(data):return
        now=time.monotonic()
        for key,(socket,_,at) in list(self.peers.items()):
            if now-at>10:socket.close();del self.peers[key]
        if peer in self.peers:
            socket,protocol,_=self.peers[peer];self.peers[peer]=(socket,protocol,now);socket.sendto(data);return
        if len(self.peers)+len(self.pending)>=16 or peer in self.pending:return
        self.pending.add(peer);asyncio.create_task(self.open(data,peer))
    async def open(self,data,peer):
        try:
            socket,protocol=await asyncio.get_running_loop().create_datagram_endpoint(lambda:Upstream(self,peer),remote_addr=(self.host,3478))
            self.peers[peer]=(socket,protocol,time.monotonic());socket.sendto(data)
        finally:self.pending.discard(peer)


async def main(host):
    socket,_=await asyncio.get_running_loop().create_datagram_endpoint(lambda:Proxy(host),local_addr=(host,3479))
    try:await asyncio.Event().wait()
    finally:socket.close()
if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--host',required=True);asyncio.run(main(p.parse_args().host))
