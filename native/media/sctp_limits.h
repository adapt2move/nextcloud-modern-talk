// SPDX-License-Identifier: EUPL-1.2
#ifndef NMT_MEDIA_SCTP_LIMITS_H_
#define NMT_MEDIA_SCTP_LIMITS_H_
#include <array>
#include <cstddef>
#include <cstdint>

namespace nmt {
// One association per helper. No heap allocation in admission. Charges for
// channel generations live until process retirement, including closed channels.
class SctpLimits {
 public:
  static constexpr size_t kPacket = 8192, kMessage = 4096;
  static constexpr size_t kQueueBytes = 65536, kFragments = 64;
  bool Packet(const uint8_t* bytes, size_t size) const;
  bool Dcep(const uint8_t* bytes, size_t size) const;
  bool Channel(const char* label, size_t size, bool ordered, bool reliable,
               bool negotiated, int sid);
  bool Fragment(uint32_t tsn, uint16_t sid, uint16_t ssn, size_t bytes,
                bool commit);
  void Release(uint32_t tsn);
  bool Complete(size_t bytes);
  void Delivered(size_t bytes);
  bool Message(size_t bytes, bool queued);
  void Unqueue(size_t bytes);
  bool Callback(size_t bytes);
  void CallbackDone(size_t bytes);
  bool DispatchBegin();
  void DispatchEnd();
  size_t fragments() const { return fragments_; }
  size_t bytes() const { return fragment_bytes_ + complete_bytes_; }
  size_t channels() const { return channels_; }
 private:
  struct FragmentEntry {
    uint32_t tsn = 0;
    uint16_t sid = 0, ssn = 0;
    size_t bytes = 0;
  };
  std::array<FragmentEntry, kFragments> entries_{};
  size_t fragments_ = 0, fragment_bytes_ = 0;
  size_t complete_messages_ = 0, complete_bytes_ = 0;
  size_t queued_messages_ = 0, queued_bytes_ = 0, channels_ = 0;
  size_t callbacks_ = 0, callback_bytes_ = 0, dispatch_depth_ = 0;
};
}  // namespace nmt
#endif
