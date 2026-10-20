// SPDX-License-Identifier: EUPL-1.2
// Exercises real admission state and malformed protocol shapes without WebRTC.
#include "sctp_limits.h"
#include <array>
#include <cassert>
#include <cstdio>
#include <cstring>

int main() {
  using nmt::SctpLimits;
  SctpLimits limits;
  const std::array<uint8_t,18> open{3,0,0,0,0,0,0,0,0,6,0,0,'s','t','a','t','u','s'};
  assert(limits.Dcep(open.data(),open.size()));
  for(size_t n=0;n<open.size();++n) assert(!limits.Dcep(open.data(),n));
  auto malformed=open;malformed[1]=0x80;assert(!limits.Dcep(malformed.data(),malformed.size()));
  malformed=open;malformed[10]=1;assert(!limits.Dcep(malformed.data(),malformed.size()));
  malformed=open;malformed[8]=0xff;assert(!limits.Dcep(malformed.data(),malformed.size()));
  malformed=open;malformed[12]='X';assert(!limits.Dcep(malformed.data(),malformed.size()));
  assert(!limits.Channel("status",6,false,true,false,0));
  assert(!limits.Channel("status",6,true,false,false,0));
  assert(!limits.Channel("status",6,true,true,true,0));
  assert(!limits.Channel("status",6,true,true,false,4));
  assert(limits.Channel("status",6,true,true,false,-1));
  assert(limits.Channel("simplewebrtc",12,true,true,false,1));
  assert(!limits.Channel("status",6,true,true,false,2));

  // Preflight does not reserve; duplicate TSNs do not grow retained fragments.
  assert(limits.Fragment(1,0,1,2048,false));assert(limits.fragments()==0);
  assert(limits.Fragment(1,0,1,2048,true));
  assert(limits.Fragment(1,0,1,2048,true));assert(limits.fragments()==1);
  assert(!limits.Fragment(1,0,1,2047,true));
  assert(limits.Fragment(2,0,1,2048,true));
  assert(!limits.Fragment(3,0,1,1,false));
  limits.Release(1);limits.Release(2);assert(limits.bytes()==0);
  for(unsigned i=0;i<16;++i)assert(limits.Fragment(100+i,0,2,1,true));
  assert(!limits.Fragment(116,0,2,1,false));
  for(unsigned i=0;i<16;++i)limits.Release(100+i);
  for(unsigned i=0;i<64;++i)assert(limits.Fragment(200+i,i%4,10+i,1,true));
  assert(!limits.Fragment(264,0,100,1,false));
  for(unsigned i=0;i<64;++i)limits.Release(200+i);
  assert(!limits.Fragment(1,4,0,1,true));
  assert(!limits.Fragment(1,0,0,4097,true));
  for(unsigned i=0;i<16;++i)assert(limits.Complete(4096));
  assert(!limits.Complete(1));assert(!limits.Fragment(1,0,0,1,true));
  for(unsigned i=0;i<16;++i)limits.Delivered(4096);
  assert(limits.bytes()==0);
  for(unsigned i=0;i<16;++i)assert(limits.Message(0,true));
  assert(!limits.Message(0,true));  // Empty callbacks cannot exhaust metadata.
  for(unsigned i=0;i<16;++i)limits.Unqueue(0);
  assert(limits.Message(4096,true));limits.Unqueue(4096);
  // Credits remain held while a callback executes and recursively queues more;
  // swapping the upstream vector must not reset either count or byte charge.
  assert(limits.DispatchBegin());assert(limits.Callback(4096));
  assert(limits.DispatchBegin());assert(!limits.DispatchBegin());
  for(unsigned i=0;i<15;++i)assert(limits.Callback(4096));
  assert(!limits.Callback(1));
  limits.DispatchEnd();assert(!limits.Callback(1));
  limits.CallbackDone(4096);assert(limits.Callback(4096));
  for(unsigned i=0;i<16;++i)limits.CallbackDone(4096);
  limits.DispatchEnd();assert(limits.DispatchBegin());limits.DispatchEnd();
  for(unsigned i=0;i<64;++i)assert(limits.Callback(0));
  assert(!limits.Callback(0));
  for(unsigned i=0;i<64;++i)limits.CallbackDone(0);
  assert(!limits.Callback(4097));assert(limits.Callback(1024));limits.CallbackDone(1024);

  std::array<uint8_t,48> packet{}; // Common header + padded DATA/DCEP.
  packet[12]=0;packet[13]=3;packet[15]=34;packet[27]=50;
  std::memcpy(packet.data()+28,open.data(),open.size());
  assert(limits.Packet(packet.data(),packet.size()));
  packet[13]|=4;assert(!limits.Packet(packet.data(),packet.size()));packet[13]=3;
  packet[21]=4;assert(!limits.Packet(packet.data(),packet.size()));packet[21]=0;
  packet[12]=64;assert(!limits.Packet(packet.data(),packet.size()));packet[12]=0;
  for(size_t n=0;n<packet.size();++n)assert(!limits.Packet(packet.data(),n));
  std::array<uint8_t,8193> oversized{};assert(!limits.Packet(oversized.data(),oversized.size()));
  // INIT with a five-byte final supported-extension parameter. Chunk length
  // excludes the three padding bytes that align the next chunk/packet end.
  std::array<uint8_t,40> init{};init[12]=1;init[15]=25;
  init[32]=0x80;init[33]=8;init[35]=5;init[36]=130;
  assert(limits.Packet(init.data(),init.size()));
  init[35]=6;assert(!limits.Packet(init.data(),init.size()));
  init[35]=4;assert(!limits.Packet(init.data(),init.size()));
  std::puts("SCTP admission: protocol, fragment, byte, count, empty-message and lifetime negatives passed");
}
