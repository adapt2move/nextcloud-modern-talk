// SPDX-License-Identifier: EUPL-1.2
#include "sctp_limits.h"
#include <algorithm>
#include <cstring>

namespace nmt {
namespace {
uint16_t U16(const uint8_t* p) { return (uint16_t(p[0]) << 8) | p[1]; }
uint32_t U32(const uint8_t* p) {
  return (uint32_t(p[0]) << 24) | (uint32_t(p[1]) << 16) |
         (uint32_t(p[2]) << 8) | p[3];
}
bool Label(const char* p, size_t n) {
  return (n == 6 && std::memcmp(p, "status", 6) == 0) ||
         (n == 12 && std::memcmp(p, "simplewebrtc", 12) == 0);
}
bool Parameters(const uint8_t* p, size_t n, bool reset) {
  unsigned count = 0;
  while (n) {
    if (n < 4 || ++count > (reset ? 4u : 16u)) return false;
    size_t size = U16(p + 2), padded = (size + 3) & ~size_t(3);
    if (size < 4 || size > n || size > (reset ? 512u : 1024u)) return false;
    if (reset) {
      uint16_t kind = U16(p);
      size_t offset = kind == 13 ? 16 : kind == 14 ? 8 : 0;
      if (offset) {
        if (size < offset || (size - offset) % 2 || size - offset > 8) return false;
        for (size_t i = offset; i < size; i += 2) if (U16(p+i) >= 4) return false;
      } else if (kind != 16 || (size != 12 && size != 20)) return false;
    }
    // RFC 9260 parameter length excludes padding. The final parameter may
    // terminate exactly at the chunk boundary; inter-parameter padding remains
    // required. Matches pinned dcSCTP Parameters::Builder/Parse, without copying.
    if (size == n) return true;
    if (padded > n) return false;
    p += padded; n -= padded;
  }
  return true;
}
}  // namespace

bool SctpLimits::Dcep(const uint8_t* p, size_t n) const {
  if (n == 1) return p[0] == 2;  // OPEN_ACK.
  // RFC 8832: ordered reliable, no protocol, only the two Talk AV labels.
  if (n < 12 || n > 24 || p[0] != 3 || p[1] != 0 || U32(p+4) != 0 ||
      U16(p+10) != 0 || size_t(U16(p+8)) != n-12) return false;
  return Label(reinterpret_cast<const char*>(p+12), n-12);
}

bool SctpLimits::Packet(const uint8_t* p, size_t n) const {
  if (n < 16 || n > kPacket) return false;
  size_t offset = 12; unsigned chunks = 0;
  while (offset < n) {
    if (n-offset < 4 || ++chunks > 16) return false;
    const auto* c = p+offset;
    size_t size = U16(c+2), padded = (size+3) & ~size_t(3);
    if (size < 4 || padded > n-offset) return false;
    switch (c[0]) {
      case 0: {  // DATA: traditional ordered messages, including DCEP.
        if (size <= 16 || size-16 > kMessage || (c[1]&4) || U16(c+8) >= 4) return false;
        auto ppid = U32(c+12);
        if (ppid == 50) {
          if ((c[1]&3) != 3 || !Dcep(c+16, size-16)) return false;
        } else if (ppid != 51 && ppid != 56) return false;
        break;
      }
      case 1: case 2:  // INIT / INIT_ACK, before native parameter vectors.
        if (size < 20 || !Parameters(c+20, size-20, false)) return false;
        break;
      case 3:  // SACK: bounded gap and duplicate lists before native vectors.
        if (size < 16 || U16(c+12) > 64 || U16(c+14) > 64 ||
            size != 16 + size_t(U16(c+12)+U16(c+14))*4) return false;
        break;
      case 4: case 5:  // HEARTBEAT / ACK.
        if (size > 256) return false;
        break;
      case 6: case 7: case 8: case 9: case 10: case 11: case 14:
        if (size > 2048) return false;
        break;
      case 130:  // RE_CONFIG: reset only, no stream-count expansion.
        if (!Parameters(c+4, size-4, true)) return false;
        break;
      default: return false;  // No I-DATA, partial reliability or extensions.
    }
    offset += padded;
  }
  return offset == n;
}

bool SctpLimits::Channel(const char* label, size_t size, bool ordered,
                         bool reliable, bool negotiated, int sid) {
  if (!Label(label,size) || !ordered || !reliable || negotiated ||
      sid < -1 || sid >= 4 || channels_ >= 2) return false;
  ++channels_;
  return true;
}

bool SctpLimits::Fragment(uint32_t tsn, uint16_t sid, uint16_t ssn,
                          size_t size, bool commit) {
  if (sid >= 4 || size == 0 || size > kMessage) return false;
  size_t bytes = size, count = 1;
  FragmentEntry* empty = nullptr;
  for (auto& entry : entries_) {
    if (!entry.bytes) { if (!empty) empty = &entry; continue; }
    if (entry.tsn == tsn) return entry.sid == sid && entry.ssn == ssn && entry.bytes == size;
    if (entry.sid == sid && entry.ssn == ssn) { bytes += entry.bytes; ++count; }
  }
  if (!empty || bytes > kMessage || count > 16 || size > kQueueBytes-this->bytes()) return false;
  if (commit) {
    *empty = {tsn,sid,ssn,size}; ++fragments_; fragment_bytes_ += size;
  }
  return true;
}
void SctpLimits::Release(uint32_t tsn) {
  for (auto& entry : entries_) if (entry.bytes && entry.tsn == tsn) {
    fragment_bytes_ -= entry.bytes; --fragments_; entry = {}; return;
  }
}
bool SctpLimits::Complete(size_t size) {
  if (size > kMessage || complete_messages_ >= 16 || size > kQueueBytes-bytes()) return false;
  ++complete_messages_; complete_bytes_ += size; return true;
}
void SctpLimits::Delivered(size_t size) {
  if (complete_messages_ && size <= complete_bytes_) { --complete_messages_; complete_bytes_ -= size; }
}
bool SctpLimits::Message(size_t size, bool queued) {
  if (size > kMessage) return false;
  if (queued) {
    if (queued_messages_ >= 16 || size > kQueueBytes-queued_bytes_) return false;
    ++queued_messages_; queued_bytes_ += size;
  }
  return true;
}
void SctpLimits::Unqueue(size_t size) {
  if (queued_messages_ && size <= queued_bytes_) { --queued_messages_; queued_bytes_ -= size; }
}
bool SctpLimits::Callback(size_t size) {
  if (size > kMessage || callbacks_ >= 64 || size > kQueueBytes-callback_bytes_) return false;
  ++callbacks_; callback_bytes_ += size; return true;
}
void SctpLimits::CallbackDone(size_t size) {
  if (callbacks_ && size <= callback_bytes_) {--callbacks_; callback_bytes_ -= size;}
}
bool SctpLimits::DispatchBegin() {
  if (dispatch_depth_ >= 2) return false;
  ++dispatch_depth_; return true;
}
void SctpLimits::DispatchEnd() { if (dispatch_depth_) --dispatch_depth_; }
}  // namespace nmt
