"""Allocation-frontier patches for the exact pinned native SCTP implementation.

The embedding exports the small C hooks. There is one association in one owned
helper; failure revokes publication independently of signaling/control work.
"""


def apply(src, modify):
    def replace(text, old, new):
        if text.count(old) != 1:
            raise RuntimeError("SCTP patch source mismatch: " + old[:100])
        return text.replace(old, new, 1)

    def declarations(text, namespace, code):
        return replace(text, "namespace " + namespace + " {", code + "\nnamespace " + namespace + " {")

    def transport(text):
        text = declarations(text, "webrtc", '''
extern "C" bool nmt_sctp_packet_allowed(const uint8_t*, size_t);
extern "C" bool nmt_sctp_send_allowed(size_t);
extern "C" void nmt_sctp_reject();
''')
        text = replace(text, '  RTC_DLOG(LS_VERBOSE) << debug_name_ << "->OnTransportReadPacket(), length="',
                       '  if (!nmt_sctp_packet_allowed(packet.payload().data(), packet.payload().size())) return;\n\n  RTC_DLOG(LS_VERBOSE) << debug_name_ << "->OnTransportReadPacket(), length="')
        text = replace(text, '  if (early_received_packets_.size() >= kMaxEarlyReceivedPackets) {\n    early_received_packets_.erase(early_received_packets_.begin());\n  }',
                       '  if (early_received_packets_.size() >= 4) { nmt_sctp_reject(); return; }')
        text = replace(text, '  StreamState stream_state;\n  stream_state.priority',
                       '  if (sid < 0 || sid >= 4 || (stream_states_.size() >= 2 && !stream_states_.contains(dcsctp::StreamID(sid)))) { nmt_sctp_reject(); return false; }\n  StreamState stream_state;\n  stream_state.priority')
        text = replace(text, '  std::vector<uint8_t> message_payload(payload.cdata(),',
                       '  if (!nmt_sctp_send_allowed(payload.size())) return RTCError(RTCErrorType::RESOURCE_EXHAUSTED);\n  std::vector<uint8_t> message_payload(payload.cdata(),')
        text = replace(text, '    socket_->SetMaxMessageSize(options.max_message_size);',
                       '    socket_->SetMaxMessageSize(std::min<size_t>(4096, options.max_message_size));')
        text = replace(text, '    data_channel_sink_->OnMaxMessageSize(options.max_message_size);',
                       '    data_channel_sink_->OnMaxMessageSize(std::min<size_t>(4096, options.max_message_size));')
        text = replace(text, '  dcsctp_options.max_message_size = options.max_message_size;',
                       '  dcsctp_options.max_message_size = std::min<size_t>(4096, options.max_message_size);')
        text = replace(text, '  dcsctp_options.max_retransmissions = std::nullopt;',
                       '  dcsctp_options.max_retransmissions = 10;')
        text = replace(text, '  dcsctp_options.max_init_retransmits = std::nullopt;',
                       '  dcsctp_options.max_init_retransmits = 8;')
        text = replace(text, '''  dcsctp_options.per_stream_send_queue_limit =
      DataChannelInterface::MaxSendQueueSize();
  dcsctp_options.announced_maximum_outgoing_streams = options.max_sctp_streams;
  // This is just set to avoid denial-of-service. Practically unlimited.
  dcsctp_options.max_send_buffer_size = std::numeric_limits<size_t>::max();
  dcsctp_options.enable_message_interleaving =
      field_trials.IsEnabled("WebRTC-DataChannelMessageInterleaving");''', '''  dcsctp_options.per_stream_send_queue_limit = 32768;
  dcsctp_options.announced_maximum_outgoing_streams = 4;
  dcsctp_options.announced_maximum_incoming_streams = 4;
  dcsctp_options.max_send_buffer_size = 65536;
  dcsctp_options.max_receiver_window_buffer_size = 65536;
  dcsctp_options.total_buffered_amount_low_threshold = 32768;
  dcsctp_options.enable_message_interleaving = false;
  dcsctp_options.enable_partial_reliability = false;''')
        return text

    def controller(text):
        text = declarations(text, "webrtc", '''
extern "C" bool nmt_sctp_dcep_allowed(const uint8_t*, size_t);
extern "C" bool nmt_sctp_channel_allowed(const char*, size_t, bool, bool, bool, int);
''')
        text = replace(text, '  // Received OPEN message; parse and signal that a new data channel should',
                       '  if (!nmt_sctp_dcep_allowed(buffer.cdata(), buffer.size())) return true;\n  // Received OPEN message; parse and signal that a new data channel should')
        text = replace(text, '''DataChannelController::CreateDataChannel(absl::string_view label,
                                         InternalDataChannelInit& config) {
  std::optional<StreamId> sid = std::nullopt;''', '''DataChannelController::CreateDataChannel(absl::string_view label,
                                         InternalDataChannelInit& config) {
  if (!config.protocol.empty() || !nmt_sctp_channel_allowed(label.data(), label.size(), config.ordered,
      !config.maxRetransmits.has_value() && !config.maxRetransmitTime.has_value(), config.negotiated, config.id))
    return RTCError(RTCErrorType::RESOURCE_EXHAUSTED, "Unsupported bounded channel profile");
  std::optional<StreamId> sid = std::nullopt;''')
        return text

    def socket(text):
        text = declarations(text, "dcsctp", '''
extern "C" bool nmt_sctp_fragment_allowed(const uint8_t*, size_t);
''')
        text = replace(text, '  std::optional<DataChunk> chunk = DataChunk::Parse(descriptor.data);',
                       '  if (!nmt_sctp_fragment_allowed(descriptor.data.data(), descriptor.data.size())) return;\n  std::optional<DataChunk> chunk = DataChunk::Parse(descriptor.data);')
        text = replace(text, '  if (send_queue_.total_buffered_amount() >= options_.max_send_buffer_size ||\n      send_queue_.buffered_amount(message.stream_id()) >=\n          options_.per_stream_send_queue_limit) {',
                       '  if (message.payload().size() > options_.max_send_buffer_size - std::min(send_queue_.total_buffered_amount(), options_.max_send_buffer_size) ||\n      message.payload().size() > options_.per_stream_send_queue_limit - std::min(send_queue_.buffered_amount(message.stream_id()), options_.per_stream_send_queue_limit)) {')
        return text

    def reassembly(text):
        text = declarations(text, "dcsctp", '''
extern "C" bool nmt_sctp_fragment_commit(uint32_t, uint16_t, uint16_t, size_t);
extern "C" void nmt_sctp_fragment_release(uint32_t);
extern "C" bool nmt_sctp_complete(size_t);
extern "C" void nmt_sctp_delivered(size_t);
extern "C" void nmt_sctp_reject();
''')
        text = replace(text, 'void ReassemblyQueue::Add(TSN tsn, Data data) {',
                       'void ReassemblyQueue::Add(TSN tsn, Data data) {\n  if (!nmt_sctp_fragment_commit(*tsn, *data.stream_id, *data.ssn, data.size())) return;')
        text = replace(text, '  queued_bytes_ -= ret.payload().size();',
                       '  queued_bytes_ -= ret.payload().size();\n  nmt_sctp_delivered(ret.payload().size());')
        text = replace(text, '  queued_bytes_ += message.payload().size();\n  reassembled_messages_.emplace_back(std::move(message));',
                       '  for (auto tsn : tsns) nmt_sctp_fragment_release(*tsn.Wrap());\n  queued_bytes_ += message.payload().size();\n  if (!nmt_sctp_complete(message.payload().size())) return;\n  reassembled_messages_.emplace_back(std::move(message));')
        # The DATA ledger bounds deferred data to 64 entries. Forward-TSN is
        # excluded by packet admission; stream reset parameters are <=4 streams.
        return text

    def channel(text):
        text = declarations(text, "webrtc", '''
extern "C" bool nmt_sctp_message_allowed(size_t, bool);
extern "C" void nmt_sctp_unqueue(size_t);
''')
        text = replace(text, '  bool binary = (type == DataMessageType::kBinary);\n  auto buffer = std::make_unique<DataBuffer>(payload, binary);',
                       '  bool binary = (type == DataMessageType::kBinary);\n  if (!nmt_sctp_message_allowed(payload.size(), !(state_ == kOpen && observer_))) return;\n  auto buffer = std::make_unique<DataBuffer>(payload, binary);')
        text = replace(text, '    std::unique_ptr<DataBuffer> buffer = queued_received_data_.PopFront();',
                       '    std::unique_ptr<DataBuffer> buffer = queued_received_data_.PopFront();\n    nmt_sctp_unqueue(buffer->size());')
        return text

    modify(src, "media/sctp/dcsctp_transport.cc", transport)
    modify(src, "pc/data_channel_controller.cc", controller)
    modify(src, "net/dcsctp/socket/dcsctp_socket.cc", socket)
    modify(src, "net/dcsctp/rx/reassembly_queue.cc", reassembly)
    modify(src, "pc/sctp_data_channel.cc", channel)
    def data_tracker(text):
        # Bound the retained gap ranges, even after completed messages leave
        # reassembly. 128 possible outstanding TSNs imply at most64 gaps.
        return replace(text, "kMaxAcceptedOutstandingFragments = 100000;",
                       "kMaxAcceptedOutstandingFragments = 128;")
    modify(src, "net/dcsctp/rx/data_tracker.h", data_tracker)

    def callbacks(text):
        text = replace(text, '#include <utility>', '#include <utility>\n#include <type_traits>')
        text = declarations(text, "dcsctp", '''
extern "C" void nmt_sctp_reject();
extern "C" bool nmt_sctp_callback_allowed(size_t);
extern "C" void nmt_sctp_callback_done(size_t);
extern "C" bool nmt_sctp_dispatch_begin();
extern "C" void nmt_sctp_dispatch_end();
''')
        text = replace(text, '  std::vector<std::pair<Callback, CallbackData>> deferred;', '''
  if (!nmt_sctp_dispatch_begin()) return;
  struct NmtDispatchGuard { ~NmtDispatchGuard() { nmt_sctp_dispatch_end(); } } nmt_dispatch_guard;
  std::vector<std::pair<Callback, CallbackData>> deferred;''')
        text = replace(text, '''  for (auto& [cb, data] : deferred) {
    cb(std::move(data), underlying_);
  }''', '''  for (auto& [cb, data] : deferred) {
    const size_t charge = std::visit([](const auto& value) -> size_t {
      using T = std::decay_t<decltype(value)>;
      if constexpr (std::is_same_v<T, DcSctpMessage>) return value.payload().size();
      else if constexpr (std::is_same_v<T, Error>) return value.message.size();
      else if constexpr (std::is_same_v<T, StreamReset>) return value.message.size() + value.streams.size()*sizeof(StreamID);
      else return 0;
    }, data);
    cb(std::move(data), underlying_);
    nmt_sctp_callback_done(charge);
  }''')
        frontiers = {
            "OnMessageReceived": "message.payload().size()",
            "OnMessageReady": "0",
            "OnError": "message.size()",
            "OnAborted": "message.size()",
            "OnConnected": "0", "OnClosed": "0", "OnConnectionRestarted": "0",
            "OnStreamsResetFailed": "reason.size() + outgoing_streams.size()*sizeof(StreamID)",
            "OnStreamsResetPerformed": "outgoing_streams.size()*sizeof(StreamID)",
            "OnIncomingStreamsReset": "incoming_streams.size()*sizeof(StreamID)",
            "OnBufferedAmountLow": "0", "OnTotalBufferedAmountLow": "0",
        }
        for name, charge in frontiers.items():
            start = text.index("void CallbackDeferrer::" + name + "(")
            end = text.index("\n}\n", start) + 3
            block = text[start:end]
            block = replace(block, "  RTC_DCHECK(prepared_);\n  deferred_.emplace_back(",
                            "  RTC_DCHECK(prepared_);\n  if (!nmt_sctp_callback_allowed(" + charge + ")) return;\n  deferred_.emplace_back(")
            text = text[:start] + block + text[end:]
        for signature, condition in [
            ('void CallbackDeferrer::OnError(ErrorKind error, absl::string_view message) {', 'message.size() > 1024'),
            ('void CallbackDeferrer::OnAborted(ErrorKind error, absl::string_view message) {', 'message.size() > 1024'),
            ('    absl::string_view reason) {', 'outgoing_streams.size() > 4 || reason.size() > 1024'),
            ('    std::span<const StreamID> outgoing_streams) {', 'outgoing_streams.size() > 4'),
            ('    std::span<const StreamID> incoming_streams) {', 'incoming_streams.size() > 4'),
        ]:
            text = replace(text, signature, signature + '\n  if (' + condition + ') { nmt_sctp_reject(); return; }')
        return text
    modify(src, "net/dcsctp/socket/callback_deferrer.cc", callbacks)
    def advertised_size(text):
        for old, new in [
            ("data->set_max_message_size(kSctpSendBufferSize);", "data->set_max_message_size(4096);"),
            ("data_answer->as_sctp()->set_max_message_size(kSctpSendBufferSize);", "data_answer->as_sctp()->set_max_message_size(4096);"),
            ("offer_data_description->max_message_size(), kSctpSendBufferSize));", "offer_data_description->max_message_size(), 4096));"),
        ]:
            text = replace(text, old, new)
        return text
    modify(src, "pc/media_session.cc", advertised_size)
