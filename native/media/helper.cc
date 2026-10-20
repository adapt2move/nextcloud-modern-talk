// SPDX-License-Identifier: EUPL-1.2
// Project-owned diagnostic embedding of the pinned upstream native API.
// Production capture is unavailable until the allocation/platform profiles qualify.
#include <algorithm>
#include <array>
#include <atomic>
#include <chrono>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <poll.h>
#include <fcntl.h>
#include <signal.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>
#include "sctp_limits.h"

#include "api/audio_codecs/audio_decoder_factory_template.h"
#include "api/audio_codecs/audio_encoder_factory_template.h"
#include "api/audio_codecs/opus/audio_decoder_opus.h"
#include "api/audio_codecs/opus/audio_encoder_opus.h"
#include "api/create_peerconnection_factory.h"
#include "api/environment/environment_factory.h"
#include "api/jsep.h"
#include "api/make_ref_counted.h"
#include "api/peer_connection_interface.h"
#include "api/rtp_transceiver_interface.h"
#include "api/set_remote_description_observer_interface.h"
#include "api/set_local_description_observer_interface.h"
#include "api/video/i420_buffer.h"
#include "api/video/video_frame.h"
#include "api/video_codecs/video_decoder_factory.h"
#include "api/video_codecs/video_encoder_factory_template.h"
#include "api/video_codecs/video_encoder_factory_template_libvpx_vp8_adapter.h"
#include "common_video/include/video_frame_buffer_pool.h"
#include "api/video/video_broadcaster.h"
#include "modules/audio_device/include/test_audio_device.h"
#include "modules/video_coding/codecs/vp8/include/vp8.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "pc/video_track_source.h"
#include "rtc_base/logging.h"
#include "rtc_base/ssl_adapter.h"
#include "rtc_base/thread.h"

namespace nmt {
using Clock = std::chrono::steady_clock;
const auto born = Clock::now();
constexpr uint64_t kSyntheticLifetimeUs = 30'000'000;
uint64_t Now() { return std::chrono::duration_cast<std::chrono::microseconds>(Clock::now() - born).count(); }
std::atomic<uint64_t> authority_until{0};
std::atomic<bool> revoked{false}, drained{false};
std::atomic<uint64_t> video_source{0}, video_decoded{0}, audio_source{0}, audio_nonzero{0};
std::atomic<uint64_t> sends{0}, blocked{0}, last_send{0}, decoder_rejected{0}, revoked_at{0};
SctpLimits sctp_limits;
std::mutex sctp_mutex;
std::atomic<uint64_t> data_opened{0}, data_closed{0}, data_messages{0}, data_bytes{0};
std::atomic<uint64_t> data_rejected{0}, negotiations{0}, signaling_state{0};
std::atomic<uint8_t> data_rejection_frontier{0};
std::atomic<unsigned> data_changes{0};
size_t data_send_messages=0, data_send_bytes=0;
constexpr size_t kRecord = 8192, kHeader = 88, kSdp = 65536, kChunk = kRecord-kHeader-9;
using Scope = std::array<uint8_t, 64>;
Scope scope{};
uint64_t Read64(const uint8_t* p) { uint64_t v=0; for(int i=0;i<8;++i) v=(v<<8)|p[i]; return v; }
uint32_t Read32(const uint8_t* p) { return uint32_t(p[0])<<24|uint32_t(p[1])<<16|uint32_t(p[2])<<8|p[3]; }
void Write64(uint8_t* p,uint64_t v) { for(int i=7;i>=0;--i) {p[i]=v&255;v>>=8;} }
void Write32(uint8_t* p,uint32_t v) { for(int i=3;i>=0;--i) {p[i]=v&255;v>>=8;} }
bool Live() {
  const auto now=Now();
  return !revoked.load(std::memory_order_acquire) && now<kSyntheticLifetimeUs
      && now<authority_until.load(std::memory_order_acquire);
}
void Revoke() {
  bool expected=false;
  if(revoked.compare_exchange_strong(expected,true,std::memory_order_acq_rel))revoked_at.store(Now());
  authority_until.store(0);
}
bool ReadExact(int fd, uint8_t* p, size_t n) {
  size_t done=0; auto deadline=Clock::now()+std::chrono::seconds(3);
  while(done<n) {
    pollfd poller{fd,POLLIN,0};
    int r=poll(&poller,1,20);
    if(Clock::now()>deadline || (revoked.load() && fd==STDIN_FILENO)) return false;
    if(r<0) return false;
    if(!r) continue;
    auto got=read(fd,p+done,n-done);
    if(got<=0) return false;
    done+=size_t(got);
  }
  return true;
}
// Output is a bounded nonblocking pipe. Backpressure closes admission; no event
// queue or executor may accumulate arbitrary SDP/candidate output.
std::mutex output_mutex;
uint64_t output_serial=0;
bool Emit(uint16_t kind,uint64_t op,const uint8_t* data=nullptr,size_t size=0) {
  if(size>kRecord-kHeader) { Revoke(); return false; }
  std::lock_guard<std::mutex> lock(output_mutex);
  std::array<uint8_t,kRecord+4> record{};
  Write32(record.data(),kHeader+size);
  memcpy(record.data()+4,"NMTM",4); record[9]=2;
  record[10]=kind>>8;record[11]=kind&255;
  memcpy(record.data()+12,scope.data(),64);
  Write64(record.data()+76,op);Write64(record.data()+84,++output_serial);
  if(size) memcpy(record.data()+92,data,size);
  size_t done=0; auto deadline=Clock::now()+std::chrono::milliseconds(30);
  while(done<size+kHeader+4) {
    pollfd p{STDOUT_FILENO,POLLOUT,0};
    if(poll(&p,1,5)<0 || Clock::now()>deadline) {Revoke();return false;}
    if(!(p.revents&POLLOUT)) continue;
    auto written=write(STDOUT_FILENO,record.data()+done,size+kHeader+4-done);
    if(written<0 && (errno==EAGAIN||errno==EWOULDBLOCK))continue;
    if(written<=0) {Revoke();return false;}
    done+=size_t(written);
  }
  return true;
}
void Failure(uint8_t reason,uint64_t op=1) { Emit(106,op,&reason,1); Revoke(); }
void Watchdog(int fd) {
  std::array<uint8_t,80> message{}; size_t have=0;uint64_t serial=0;
  auto first_deadline=Clock::now()+std::chrono::seconds(3);
  while(!revoked.load()) {
    if(Now()>=kSyntheticLifetimeUs) {Revoke();break;}
    pollfd p{fd,POLLIN,0};int status=poll(&p,1,5);
    if(status<0 || (p.revents&(POLLHUP|POLLERR|POLLNVAL))) {Revoke();break;}
    if(status>0 && (p.revents&POLLIN)) {
      auto count=read(fd,message.data()+have,message.size()-have);
      if(count<=0) {Revoke();break;}
      have+=size_t(count);
      if(have==message.size()) {
        auto ttl=Read32(message.data()+68);auto next=Read64(message.data()+72);
        if(memcmp(message.data(),"NMW1",4)||memcmp(message.data()+4,scope.data(),64)
           || !ttl || ttl>100 || next<=serial) {Revoke();break;}
        serial=next;have=0;
        // A late renewal cannot revive an expired generation.
        if(authority_until.load()!=0 && !Live()) {Revoke();break;}
        authority_until.store(std::min(kSyntheticLifetimeUs,Now()+uint64_t(ttl)*1000),std::memory_order_release);
      }
    }
    if((authority_until.load()!=0 && !Live()) || (serial==0 && Clock::now()>first_deadline)) Revoke();
  }
  close(fd);
  // Independent of signaling, capture, stdout and ordinary IPC. This is an
  // escalation, never an independent assertion of physical device release.
  const auto deadline=Clock::now()+std::chrono::milliseconds(200);
  while(!drained.load() && Clock::now()<deadline) std::this_thread::sleep_for(std::chrono::milliseconds(2));
  if(!drained.load()) _exit(70);
}

class Tone final : public webrtc::TestAudioDeviceModule::Capturer {
 public:
  int SamplingFrequency() const override {return 48000;}
  int NumChannels() const override {return 1;}
  bool Capture(webrtc::BufferT<int16_t>* buffer) override {
    buffer->SetSize(480);
    for(size_t i=0;i<480;++i) (*buffer)[i]=Live()?int16_t(6000*std::sin((phase_++%48000)*2*3.141592653589793*440/48000)):0;
    if(Live()) ++audio_source;
    return !revoked.load();
  }
 private: uint64_t phase_=0;
};
class AudioSink final : public webrtc::TestAudioDeviceModule::Renderer {
 public:
  int SamplingFrequency() const override {return 48000;}
  int NumChannels() const override {return 1;}
  bool Render(std::span<const int16_t> samples) override {
    if(samples.size()>480) {Revoke();return false;}
    if(std::any_of(samples.begin(),samples.end(),[](int16_t s){return s>100||s<-100;})) ++audio_nonzero;
    return !revoked.load();
  }
};
class SyntheticCamera : public webrtc::VideoTrackSource {
 public:
  SyntheticCamera():VideoTrackSource(false) {}
  void Start() {
    worker_=std::thread([this]{
      // Four buffers are created lazily and reused only after refcounts settle.
      // This bounds this source, not upstream codec internal allocations.
      webrtc::VideoFrameBufferPool pool(true,4);
      auto next=Clock::now();
      while(!stop_.load()&&!revoked.load()) {
        if(Live()) {
          auto buffer=pool.CreateI420Buffer(640,360);
          if(buffer) {
            for(int y=0;y<360;++y) for(int x=0;x<640;++x)
              buffer->MutableDataY()[y*buffer->StrideY()+x]=uint8_t(32+((x+y+video_source.load())%180));
            for(int y=0;y<180;++y) {
              memset(buffer->MutableDataU()+y*buffer->StrideU(),96,320);
              memset(buffer->MutableDataV()+y*buffer->StrideV(),160,320);
            }
            broadcaster_.OnFrame(webrtc::VideoFrame::Builder().set_video_frame_buffer(buffer).set_timestamp_us(Now()).build());
            ++video_source;
          }
        }
        next+=std::chrono::milliseconds(50);
        std::this_thread::sleep_until(next);
      }
    });
  }
  void Stop() {stop_.store(true);if(worker_.joinable())worker_.join();}
 protected:
  ~SyntheticCamera() override {Stop();}
  webrtc::VideoSourceInterface<webrtc::VideoFrame>* source() override {return &broadcaster_;}
 private:
  webrtc::VideoBroadcaster broadcaster_;std::thread worker_;std::atomic<bool> stop_{false};
};
class VideoSink final : public webrtc::VideoSinkInterface<webrtc::VideoFrame> {
 public: void OnFrame(const webrtc::VideoFrame& frame) override {
    if(frame.width()>1280||frame.height()>720) {Revoke();return;}
    ++video_decoded;
  }
};

class CameraDecoder final : public webrtc::VideoDecoder {
 public:
  explicit CameraDecoder(const webrtc::Environment& env):inner_(webrtc::CreateVp8Decoder(env)) {}
  bool Configure(const Settings& input) override {
    auto settings=input; settings.set_buffer_pool_size(8); settings.set_number_of_cores(2);
    settings.set_max_render_resolution(webrtc::RenderResolution(1280,720));
    return inner_->Configure(settings);
  }
  int32_t Decode(const webrtc::EncodedImage& image,int64_t time) override {
    if(!Live()||image.size()>1024*1024||image.size()<3) return Reject();
    const auto* p=image.data();
    if((p[0]&1)==0) {
      if(image.size()<10||p[3]!=0x9d||p[4]!=1||p[5]!=0x2a) return Reject();
      unsigned w=(p[6]|unsigned(p[7])<<8)&0x3fff,h=(p[8]|unsigned(p[9])<<8)&0x3fff;
      if(!w||!h||w>1280||h>720||(w&1)||(h&1)) return Reject();
      keyframe_seen_=true;
    } else if(!keyframe_seen_) return Reject();
    return inner_->Decode(image,time);
  }
  int32_t RegisterDecodeCompleteCallback(webrtc::DecodedImageCallback* cb) override {return inner_->RegisterDecodeCompleteCallback(cb);}
  int32_t Release() override {keyframe_seen_=false;return inner_->Release();}
  DecoderInfo GetDecoderInfo() const override {return inner_->GetDecoderInfo();}
 private:
  int32_t Reject() {if(++decoder_rejected>=3)Revoke();return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;}
  std::unique_ptr<webrtc::VideoDecoder> inner_;bool keyframe_seen_=false;
};
class DecoderFactory final : public webrtc::VideoDecoderFactory {
 public:
  std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override {return {webrtc::SdpVideoFormat::VP8()};}
  std::unique_ptr<webrtc::VideoDecoder> Create(const webrtc::Environment& env,const webrtc::SdpVideoFormat& format) override {
    if(format.name!="VP8"||count_++!=0)return nullptr;
    return std::make_unique<CameraDecoder>(env);
  }
 private:unsigned count_=0;
};

// Restrict before encoder initialization or reconfiguration allocates backing.
// Internal libvpx reference/work allocations still require a charging hook.
class CameraEncoder final : public webrtc::VideoEncoder {
 public:
  explicit CameraEncoder(const webrtc::Environment& env):inner_(webrtc::CreateVp8Encoder(env)) {}
  int InitEncode(const webrtc::VideoCodec* codec,const Settings& input) override {
    if(!codec||codec->codecType!=webrtc::kVideoCodecVP8||!codec->width||!codec->height
       ||codec->width>1280||codec->height>720||codec->numberOfSimulcastStreams>1
       ||codec->VP8().numberOfTemporalLayers>1)return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    auto settings=input;settings.number_of_cores=2;settings.encoder_thread_limit=1;
    return inner_->InitEncode(codec,settings);
  }
  int32_t RegisterEncodeCompleteCallback(webrtc::EncodedImageCallback* callback) override {return inner_->RegisterEncodeCompleteCallback(callback);}
  int32_t Release() override {return inner_->Release();}
  int32_t Encode(const webrtc::VideoFrame& frame,const std::vector<webrtc::VideoFrameType>* types) override {
    if(!Live()||frame.width()>1280||frame.height()>720)return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
    return inner_->Encode(frame,types);
  }
  void SetRates(const RateControlParameters& parameters) override {inner_->SetRates(parameters);}
  void OnPacketLossRateUpdate(float rate) override {inner_->OnPacketLossRateUpdate(rate);}
  void OnRttUpdate(int64_t value) override {inner_->OnRttUpdate(value);}
  void OnLossNotification(const LossNotification& value) override {inner_->OnLossNotification(value);}
  EncoderInfo GetEncoderInfo() const override {return inner_->GetEncoderInfo();}
 private:std::unique_ptr<webrtc::VideoEncoder> inner_;
};
struct OneLayerVp8Adapter {
  static std::vector<webrtc::SdpVideoFormat> SupportedFormats() {
    return {webrtc::SdpVideoFormat(webrtc::SdpVideoFormat::VP8(),{webrtc::ScalabilityMode::kL1T1})};
  }
  static std::unique_ptr<webrtc::VideoEncoder> CreateEncoder(const webrtc::Environment& env,const webrtc::SdpVideoFormat&) {
    return std::make_unique<CameraEncoder>(env);
  }
  static bool IsScalabilityModeSupported(webrtc::ScalabilityMode mode) {return mode==webrtc::ScalabilityMode::kL1T1;}
};

class Session;
class StatusObserver final : public webrtc::DataChannelObserver {
 public:
  // No payload queue, API reentry or blocking callback. All user content is
  // ignored here; status messages never grant local capture/publication.
  bool IsOkToCallOnTheNetworkThread() override {return true;}
  void OnStateChange() override {data_changes.fetch_add(1);}
  void OnMessage(const webrtc::DataBuffer& data) override {
    if(data.binary||data.size()>SctpLimits::kMessage||++data_messages>2048) {
      ++data_rejected;Revoke();return;
    }
    data_bytes.fetch_add(data.size());
  }
};
class LocalSet : public webrtc::SetLocalDescriptionObserverInterface {
 public: explicit LocalSet(Session* owner):owner_(owner){}
  void OnSetLocalDescriptionComplete(webrtc::RTCError error) override;
 private:Session* owner_;
};
class RemoteSet : public webrtc::SetRemoteDescriptionObserverInterface {
 public: RemoteSet(Session* owner,bool answer):owner_(owner),answer_(answer){}
  void OnSetRemoteDescriptionComplete(webrtc::RTCError error) override;
 private:Session* owner_;bool answer_;
};
class Created : public webrtc::CreateSessionDescriptionObserver {
 public:explicit Created(Session* owner):owner_(owner){}
  void OnSuccess(webrtc::SessionDescriptionInterface* desc) override;
  void OnFailure(webrtc::RTCError) override {Failure(3);}
 private:Session* owner_;
};
class Session final : public webrtc::PeerConnectionObserver {
 public:
  bool Init(uint8_t intent) {
    if(intent>3)return false;
    intent_=intent;
    network_=webrtc::Thread::CreateWithSocketServer(); network_->SetName("nmt-network",nullptr);network_->Start();
    signaling_=webrtc::Thread::Create();signaling_->SetName("nmt-signaling",nullptr);signaling_->Start();
    return signaling_->BlockingCall([this]{
      auto env=webrtc::CreateEnvironment();
      adm_=webrtc::TestAudioDeviceModule::Create(env,(intent_&1)?std::make_unique<Tone>():nullptr,std::make_unique<AudioSink>());
      factory_=webrtc::CreatePeerConnectionFactory(network_.get(),signaling_.get(),adm_,
        webrtc::CreateAudioEncoderFactory<webrtc::AudioEncoderOpus>(),
        webrtc::CreateAudioDecoderFactory<webrtc::AudioDecoderOpus>(),
        std::make_unique<webrtc::VideoEncoderFactoryTemplate<OneLayerVp8Adapter>>(),
        std::make_unique<DecoderFactory>(),nullptr,nullptr);
      if(!factory_)return false;
      webrtc::PeerConnectionInterface::RTCConfiguration config;
      config.sdp_semantics=webrtc::SdpSemantics::kUnifiedPlan;
      config.bundle_policy=webrtc::PeerConnectionInterface::kBundlePolicyMaxBundle;
      config.rtcp_mux_policy=webrtc::PeerConnectionInterface::kRtcpMuxPolicyRequire;
      config.max_sctp_streams=4;
      auto result=factory_->CreatePeerConnectionOrError(config,webrtc::PeerConnectionDependencies(this));
      if(!result.ok())return false;
      pc_=result.MoveValue();
      if(intent_&1) {
        webrtc::AudioOptions options; options.echo_cancellation=false;options.noise_suppression=false;options.auto_gain_control=false;
        auto audio=factory_->CreateAudioTrack("audio",factory_->CreateAudioSource(options).get());
        if(!pc_->AddTrack(audio,{"nmt"}).ok())return false;
      }
      if(intent_&2) {
        camera_=webrtc::make_ref_counted<SyntheticCamera>();
        auto track=factory_->CreateVideoTrack(camera_,"video");
        if(!pc_->AddTrack(track,{"nmt"}).ok())return false;
        camera_->Start();
      }
      return true;
    });
  }
  void Offer(uint64_t op) {signaling_->BlockingCall([this,op]{
    if((phase_!=Phase::Initial&&phase_!=Phase::Stable)||!Live()||local_offers_>=2){Failure(13);return;}
    if(phase_==Phase::Initial) {
      webrtc::DataChannelInit init;
      auto channel=pc_->CreateDataChannelOrError("status",&init);
      if(!channel.ok()){Failure(7);return;}
      OnDataChannel(channel.MoveValue());
    }
    ++local_offers_;phase_=Phase::MakingOffer;operation_=op;
    pc_->CreateOffer(webrtc::make_ref_counted<Created>(this).get(),{});
  });}
  void Answer() {if(Live())pc_->CreateAnswer(webrtc::make_ref_counted<Created>(this).get(),{});}
  void Local(webrtc::SessionDescriptionInterface* raw) {
    std::unique_ptr<webrtc::SessionDescriptionInterface> desc(raw);
    if(!Live())return;
    pc_->SetLocalDescription(std::move(desc),webrtc::make_ref_counted<LocalSet>(this));
  }
  void EmitDescription() {
    if(!Live()||!pc_->local_description())return;
    std::string sdp;
    if(!pc_->local_description()->ToString(&sdp)||sdp.size()>kSdp) {Failure(4,operation_);return;}
    uint8_t kind=pc_->local_description()->GetType()==webrtc::SdpType::kOffer?1:2;
    phase_=kind==1?Phase::AwaitingAnswer:Phase::Stable;
    if(kind==2)++negotiations;
    for(size_t offset=0;offset<sdp.size();offset+=kChunk) {
      std::array<uint8_t,kRecord-kHeader> p{};size_t count=std::min(kChunk,sdp.size()-offset);
      Write32(p.data(),sdp.size());Write32(p.data()+4,offset);p[8]=kind;
      memcpy(p.data()+9,sdp.data()+offset,count);
      if(!Emit(101,operation_,p.data(),count+9))break;
    }
  }
  bool Remote(uint8_t kind,const std::string& sdp,uint64_t op) {
    // Count before the native SDP parser may allocate transceivers/candidates.
    unsigned media=0,candidates=0,audio=0,video=0,application=0;size_t begin=0;
    std::array<uint8_t,3> types{};std::array<std::string,3> mids{};
    std::string transport_identity;
    while(begin<sdp.size()) {
      size_t end=sdp.find('\n',begin);if(end==std::string::npos)end=sdp.size();
      if(end-begin>2048){Failure(14);return false;}
      if(sdp.compare(begin,2,"m=")==0) {
        if(++media>3){Failure(15);return false;}
        if(sdp.compare(begin,8,"m=audio ")==0 && ++audio==1)types[media-1]=1;
        else if(sdp.compare(begin,8,"m=video ")==0 && ++video==1)types[media-1]=2;
        else if(sdp.compare(begin,14,"m=application ")==0 && ++application==1)types[media-1]=3;
        else {Failure(15);return false;}
      }
      if(sdp.compare(begin,6,"a=mid:")==0) {
        size_t length=end-begin-6;if(length&&sdp[end-1]=='\r')--length;
        if(!media||!length||length>64||!mids[media-1].empty()){Failure(15);return false;}
        mids[media-1]=sdp.substr(begin+6,length);
      }
      if(sdp.compare(begin,12,"a=candidate:")==0 && ++candidates>64){Failure(16);return false;}
      if(sdp.compare(begin,10,"a=simulcast")==0 || sdp.compare(begin,6,"a=rid:")==0){Failure(17);return false;}
      if(sdp.compare(begin,12,"a=sctp-init:")==0){Failure(20);return false;}
      if(sdp.compare(begin,12,"a=ice-ufrag:")==0||sdp.compare(begin,10,"a=ice-pwd:")==0||sdp.compare(begin,14,"a=fingerprint:")==0) {
        if(transport_identity.size()+end-begin+1>2048){Failure(20);return false;}
        transport_identity.append(sdp,begin,end-begin);transport_identity.push_back('\n');
      }
      begin=end+1;
    }
    if(!audio||!video){Failure(15);return false;}
    for(unsigned i=0;i<media;++i) {
      if(mids[i].empty()){Failure(15);return false;}
      for(unsigned j=0;j<i;++j)if(mids[i]==mids[j]){Failure(15);return false;}
    }
    return signaling_->BlockingCall([this,kind,sdp,op,types,mids,media,transport_identity,candidates]{
      if(!Live()||remote_descriptions_>=2||
         (kind==1&&phase_!=Phase::Initial&&phase_!=Phase::Stable)||
         (kind==2&&phase_!=Phase::AwaitingAnswer))return false;
      // Renegotiation may reuse the admitted AV/application transports. ICE/
      // fingerprint changes or new m-sections require a fresh owner admission.
      if(remote_descriptions_ && (mids!=media_mids_||types!=media_types_||transport_identity!=remote_transport_identity_))return false;
      if(remote_candidates_+candidates>64)return false;
      remote_candidates_+=candidates;++remote_descriptions_;operation_=op;
      media_types_=types;media_mids_=mids;remote_media_=media;
      remote_transport_identity_=transport_identity;phase_=Phase::ApplyingRemote;
      auto desc=webrtc::CreateSessionDescription(kind==1?webrtc::SdpType::kOffer:webrtc::SdpType::kAnswer,sdp);
      if(!desc)return false;
      pc_->SetRemoteDescription(std::move(desc),webrtc::make_ref_counted<RemoteSet>(this,kind==1));return true;
    });
  }
  void RemoteComplete(bool answer) {
    if(!Live())return;
    if(answer)Answer();else {phase_=Phase::Stable;++negotiations;}
  }
  bool Candidate(const uint8_t* p,size_t n) {
    if(n<3||!p[0]||p[0]>64||p[1]>=remote_media_||n<=size_t(p[0])+2||n-size_t(p[0])-2>2048||++remote_candidates_>64)return false;
    std::string mid(reinterpret_cast<const char*>(p+2),p[0]);
    if(mid!=media_mids_[p[1]])return false;
    std::string value(reinterpret_cast<const char*>(p+2+p[0]),n-2-p[0]);
    return signaling_->BlockingCall([this,mid,value,index=p[1]]{
      auto c=webrtc::IceCandidate::Create(mid,index,value);return Live()&&c&&pc_->AddIceCandidate(c.get());
    });
  }
  void Close() {
    if(camera_)camera_->Stop();
    if(signaling_)signaling_->BlockingCall([this]{
      if(remote_video_)remote_video_->RemoveSink(&sink_);
      remote_video_=nullptr;
      phase_=Phase::Closing;
      for(auto& channel:channels_) if(channel) {channel->UnregisterObserver();channel->Close();channel=nullptr;}
      if(pc_)pc_->Close();pc_=nullptr;camera_=nullptr;factory_=nullptr;
      if(adm_){adm_->StopRecording();adm_->StopPlayout();adm_->Terminate();adm_=nullptr;}
    });
    if(signaling_)signaling_->Stop();if(network_)network_->Stop();
    drained.store(true);
  }
  ~Session() override {if(!drained.load())Close();}
  void OnSignalingChange(webrtc::PeerConnectionInterface::SignalingState state) override {signaling_state.store(static_cast<uint64_t>(state));}
  void OnDataChannel(webrtc::scoped_refptr<webrtc::DataChannelInterface> channel) override {
    if(!Live()||channel_count_>=channels_.size()){channel->Close();Failure(7);return;}
    auto index=channel_count_++;channels_[index]=channel;
    channel->RegisterObserver(&status_observers_[index]);data_changes.fetch_add(1);
  }
  void CheckDataStates() {
    if(!data_changes.exchange(0)||!signaling_||drained.load())return;
    signaling_->BlockingCall([this]{
      for(size_t i=0;i<channel_count_;++i)if(channels_[i]) {
        auto state=channels_[i]->state();
        if(state==webrtc::DataChannelInterface::kOpen&&!channel_opened_[i]) {channel_opened_[i]=true;++data_opened;}
        if(state==webrtc::DataChannelInterface::kClosed&&!channel_closed_[i]) {channel_closed_[i]=true;++data_closed;}
      }
    });
  }
  void OnIceGatheringChange(webrtc::PeerConnectionInterface::IceGatheringState) override {}
  void OnIceCandidate(const webrtc::IceCandidate* candidate) override {
    if(!Live())return;
    std::string line;candidate->ToString(&line);auto mid=candidate->sdp_mid();
    if(line.size()>2048||mid.size()>64||++local_candidates_>64){Failure(8);return;}
    std::array<uint8_t,2114> p{};p[0]=mid.size();p[1]=candidate->sdp_mline_index();
    memcpy(p.data()+2,mid.data(),mid.size());memcpy(p.data()+2+mid.size(),line.data(),line.size());
    Emit(102,operation_,p.data(),2+mid.size()+line.size());
  }
  void OnConnectionChange(webrtc::PeerConnectionInterface::PeerConnectionState state) override {
    if(state==webrtc::PeerConnectionInterface::PeerConnectionState::kConnected)Emit(103,operation_);
    if(state==webrtc::PeerConnectionInterface::PeerConnectionState::kFailed)Failure(9);
  }
  void OnTrack(webrtc::scoped_refptr<webrtc::RtpTransceiverInterface> transceiver) override {
    auto track=transceiver->receiver()->track();
    if(track&&track->kind()==webrtc::MediaStreamTrackInterface::kVideoKind) {
      if(remote_video_){if(remote_video_.get()!=track.get())Failure(10);return;}
      remote_video_=static_cast<webrtc::VideoTrackInterface*>(track.get());
      remote_video_->AddOrUpdateSink(&sink_,{});
    }
  }
 private:
  uint8_t intent_=0;uint64_t operation_=1;unsigned local_candidates_=0,remote_candidates_=0;
  enum class Phase {Initial,MakingOffer,AwaitingAnswer,ApplyingRemote,Stable,Closing};
  Phase phase_=Phase::Initial;
  unsigned local_offers_=0,remote_descriptions_=0;
  std::string remote_transport_identity_;
  size_t channel_count_=0;
  std::array<webrtc::scoped_refptr<webrtc::DataChannelInterface>,2> channels_{};
  std::array<StatusObserver,2> status_observers_{};
  std::array<bool,2> channel_opened_{},channel_closed_{};
  unsigned remote_media_=0;std::array<uint8_t,3> media_types_{};std::array<std::string,3> media_mids_{};
  std::unique_ptr<webrtc::Thread> network_,signaling_;
  webrtc::scoped_refptr<webrtc::AudioDeviceModule> adm_;
  webrtc::scoped_refptr<webrtc::PeerConnectionFactoryInterface> factory_;
  webrtc::scoped_refptr<webrtc::PeerConnectionInterface> pc_;
  webrtc::scoped_refptr<SyntheticCamera> camera_;
  webrtc::scoped_refptr<webrtc::VideoTrackInterface> remote_video_;VideoSink sink_;
};
void Created::OnSuccess(webrtc::SessionDescriptionInterface* desc) {owner_->Local(desc);}
void LocalSet::OnSetLocalDescriptionComplete(webrtc::RTCError e) {if(!e.ok())Failure(11);else owner_->EmitDescription();}
void RemoteSet::OnSetRemoteDescriptionComplete(webrtc::RTCError e) {if(!e.ok())Failure(12);else owner_->RemoteComplete(answer_);}

void Counters(uint64_t op) {
  std::array<uint8_t,144> p{};
  size_t channels=0;{std::lock_guard<std::mutex> lock(sctp_mutex);channels=sctp_limits.channels();}
  std::array<uint64_t,18> values{video_source.load(),video_decoded.load(),audio_source.load(),audio_nonzero.load(),
    sends.load(),blocked.load(),last_send.load(),decoder_rejected.load(),Now(),revoked_at.load(),
    channels,data_opened.load(),data_closed.load(),data_messages.load(),data_bytes.load(),data_rejected.load(),negotiations.load(),signaling_state.load()};
  for(size_t i=0;i<values.size();++i)Write64(p.data()+8*i,values[i]);
  Emit(104,op,p.data(),p.size());
}
int Run(const char* watchdog_path) {
  std::array<uint8_t,kRecord> record{};std::array<uint8_t,4> size{};
  if(!ReadExact(STDIN_FILENO,size.data(),4))return 64;
  auto n=Read32(size.data());
  if(n!=kHeader+1||!ReadExact(STDIN_FILENO,record.data(),n)||memcmp(record.data(),"NMTM",4)
     ||record[4]!=0||record[5]!=2||record[6]!=0||record[7]!=1)return 64;
  memcpy(scope.data(),record.data()+8,64);
  uint64_t op=Read64(record.data()+72),serial=Read64(record.data()+80);
  if(!op||!serial||record[kHeader]>3)return 64;
  int fd=socket(AF_UNIX,SOCK_STREAM,0);if(fd<0)return 65;
  sockaddr_un address{};address.sun_family=AF_UNIX;
  if(strlen(watchdog_path)>=sizeof(address.sun_path))return 65;
  strcpy(address.sun_path,watchdog_path);
  if(connect(fd,reinterpret_cast<sockaddr*>(&address),sizeof(address))<0)return 65;
  std::thread watchdog([fd]{Watchdog(fd);});
  auto wait_until=Clock::now()+std::chrono::seconds(2);
  while(!Live()&&!revoked.load()&&Clock::now()<wait_until)std::this_thread::sleep_for(std::chrono::milliseconds(1));
  Session session;
  if(!Live()||!session.Init(record[kHeader])) {Revoke();session.Close();watchdog.join();return 66;}
  Emit(100,op);
  std::unique_ptr<std::array<uint8_t,kSdp>> sdp;
  uint32_t total=0,received=0;uint64_t sdp_op=0;uint8_t sdp_kind=0;
  unsigned commands=0;
  while(!revoked.load()&&commands<512) {
    session.CheckDataStates();
    pollfd p{STDIN_FILENO,POLLIN,0};int status=poll(&p,1,10);
    if(status<0||(p.revents&(POLLHUP|POLLERR|POLLNVAL)))break;
    if(!status)continue;
    if(!ReadExact(STDIN_FILENO,size.data(),4))break;n=Read32(size.data());++commands;
    if(n<kHeader||n>kRecord||!ReadExact(STDIN_FILENO,record.data(),n))break;
    auto next_op=Read64(record.data()+72),next_serial=Read64(record.data()+80);
    if(memcmp(record.data(),"NMTM",4)||record[4]||record[5]!=2||record[6]
       ||memcmp(record.data()+8,scope.data(),64)||next_serial<=serial||next_op<op)break;
    op=next_op;serial=next_serial;const auto* payload=record.data()+kHeader;size_t len=n-kHeader;
    uint8_t kind=record[7];
    if(kind==2&&len==0&&!sdp)session.Offer(op);
    else if(kind==3&&len>9) {
      auto count=Read32(payload),offset=Read32(payload+4);
      if(count<5||count>kSdp||offset>count||len-9>count-offset||payload[8]<1||payload[8]>2)break;
      if(!sdp) {if(offset!=0)break;sdp=std::make_unique<std::array<uint8_t,kSdp>>();total=count;received=0;sdp_op=op;sdp_kind=payload[8];}
      if(offset!=received||count!=total||sdp_op!=op||sdp_kind!=payload[8])break;
      memcpy(sdp->data()+offset,payload+9,len-9);received+=len-9;
      if(received==total) {
        if(memcmp(sdp->data(),"v=0\r\n",5)||memchr(sdp->data(),0,total))break;
        if(!session.Remote(sdp_kind,std::string(reinterpret_cast<char*>(sdp->data()),total),op)){Failure(18);break;}
        sdp.reset();
      }
    } else if(kind==4) {if(!session.Candidate(payload,len)){Failure(19);break;}}
    else if(kind==5&&len==0)break;
    else if(kind==6&&len==0)Counters(op);
    else if(kind==7&&len==0)std::this_thread::sleep_for(std::chrono::seconds(2));
    else break;
  }
  if(data_rejected.load()) {uint8_t reason=23+data_rejection_frontier.load();Emit(106,op,&reason,1);}
  Revoke();session.Close();Counters(op);Emit(105,op);watchdog.join();return 0;
}
} // namespace nmt

// Called by the project-owned patch at PhysicalSocket::Send/SendTo. All media,
// retransmissions and TURN TCP/UDP writes using that socket backend traverse it.
extern "C" bool nmt_media_network_allowed() {bool allowed=nmt::Live();if(!allowed)++nmt::blocked;return allowed;}
extern "C" void nmt_media_network_sent() {++nmt::sends;nmt::last_send.store(nmt::Now());}
namespace {
bool SctpChecked(bool allowed,uint8_t frontier) {
  if(!allowed) {
    ++nmt::data_rejected;
    uint8_t empty=0;nmt::data_rejection_frontier.compare_exchange_strong(empty,frontier);
    nmt::Revoke();
  }
  return allowed;
}
}
extern "C" void nmt_sctp_reject() {SctpChecked(false,8);}
extern "C" bool nmt_sctp_packet_allowed(const uint8_t* p,size_t n) {
  if(!nmt::Live())return false;
  return SctpChecked(nmt::sctp_limits.Packet(p,n),1);
}
extern "C" bool nmt_sctp_dcep_allowed(const uint8_t* p,size_t n) {
  if(!nmt::Live())return false;
  return SctpChecked(nmt::sctp_limits.Dcep(p,n),2);
}
extern "C" bool nmt_sctp_channel_allowed(const char* p,size_t n,bool ordered,bool reliable,bool negotiated,int sid) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  return nmt::Live()&&SctpChecked(nmt::sctp_limits.Channel(p,n,ordered,reliable,negotiated,sid),3);
}
extern "C" bool nmt_sctp_fragment_allowed(const uint8_t* p,size_t n) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  if(!nmt::Live())return false;
  return SctpChecked(n>16&&nmt::sctp_limits.Fragment(nmt::Read32(p+4),uint16_t(p[8])*256+p[9],uint16_t(p[10])*256+p[11],n-16,false),4);
}
extern "C" bool nmt_sctp_fragment_commit(uint32_t tsn,uint16_t sid,uint16_t ssn,size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  return nmt::Live()&&SctpChecked(nmt::sctp_limits.Fragment(tsn,sid,ssn,bytes,true),4);
}
extern "C" void nmt_sctp_fragment_release(uint32_t tsn) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);nmt::sctp_limits.Release(tsn);
}
extern "C" bool nmt_sctp_complete(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  return nmt::Live()&&SctpChecked(nmt::sctp_limits.Complete(bytes),5);
}
extern "C" void nmt_sctp_delivered(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);nmt::sctp_limits.Delivered(bytes);
}
extern "C" bool nmt_sctp_message_allowed(size_t bytes,bool queued) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  return nmt::Live()&&SctpChecked(nmt::sctp_limits.Message(bytes,queued),6);
}
extern "C" void nmt_sctp_unqueue(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);nmt::sctp_limits.Unqueue(bytes);
}
extern "C" bool nmt_sctp_callback_allowed(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  return SctpChecked(nmt::sctp_limits.Callback(bytes),9);
}
extern "C" void nmt_sctp_callback_done(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);nmt::sctp_limits.CallbackDone(bytes);
}
extern "C" bool nmt_sctp_dispatch_begin() {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);return SctpChecked(nmt::sctp_limits.DispatchBegin(),9);
}
extern "C" void nmt_sctp_dispatch_end() {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);nmt::sctp_limits.DispatchEnd();
}
extern "C" bool nmt_sctp_send_allowed(size_t bytes) {
  std::lock_guard<std::mutex> lock(nmt::sctp_mutex);
  if(!nmt::Live())return false;
  if(!SctpChecked(bytes<=4096&&nmt::data_send_messages<16&&bytes<=65536-nmt::data_send_bytes,7))return false;
  ++nmt::data_send_messages;nmt::data_send_bytes+=bytes;return true;
}
int main(int argc,char** argv) {
  if(argc==2&&std::string(argv[1])=="--version") {
    const char* version="nmt-media-ipc/2 libwebrtc/a306485b7fc0fe38dd2651420e383f92f196cd77 bounded-sctp diagnostic-synthetic-only\n";
    return write(STDOUT_FILENO,version,strlen(version))>0?0:1;
  }
  if(argc!=3||std::string(argv[1])!="--diagnostic-synthetic")return 64;
  signal(SIGPIPE,SIG_IGN);
  if(fcntl(STDOUT_FILENO,F_SETFL,fcntl(STDOUT_FILENO,F_GETFL)|O_NONBLOCK)<0)return 65;
  webrtc::LogMessage::LogToDebug(webrtc::LS_NONE);webrtc::LogMessage::SetLogToStderr(false);
  if(!webrtc::InitializeSSL())return 65;
  int result=nmt::Run(argv[2]);webrtc::CleanupSSL();return result;
}
