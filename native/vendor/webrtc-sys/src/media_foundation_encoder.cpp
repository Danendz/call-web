// Hardware H.264 through Media Foundation. Windows has no WebRTC encoder that
// reaches the GPU's video engine (NVENC here is Linux only), so without this a
// Windows call encodes VP8 in software, the call's biggest CPU cost.
//
// Hardware encoder MFTs are asynchronous: they ask for input and announce
// output through events. A thread per encoder waits on those events, counts
// the requests for input that Encode() spends, and hands each encoded frame to
// WebRTC as it comes, like VideoToolbox's callback on macOS.

#if defined(_WIN32)

#include "livekit/media_foundation_encoder.h"

#include <algorithm>
#include <cmath>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "api/video/encoded_image.h"
#include "api/video/i420_buffer.h"
#include "api/video/video_codec_type.h"
#include "api/video_codecs/h264_profile_level_id.h"
#include "api/video_codecs/sdp_video_format.h"
#include "api/video_codecs/video_encoder.h"
#include "common_video/h264/h264_bitstream_parser.h"
#include "common_video/h264/h264_common.h"
#include "modules/video_coding/codecs/h264/include/h264.h"
#include "modules/video_coding/include/video_codec_interface.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "rtc_base/logging.h"
#include "third_party/libyuv/include/libyuv/convert_from.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <codecapi.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <strmif.h>
#include <wrl/client.h>

namespace livekit_ffi {

namespace {

using Microsoft::WRL::ComPtr;

// libwebrtc's own H.264 thresholds: above the high one the quality scaler
// lowers the resolution, below the low one it raises it again
constexpr int kLowQp = 24;
constexpr int kHighQp = 37;
// Frames waiting for their output. A hardware encoder holds one or two; past
// this something is stuck, and the oldest are forgotten.
constexpr size_t kMaxPending = 30;

std::string Utf8(const wchar_t* text) {
  if (!text) {
    return {};
  }
  const int size =
      WideCharToMultiByte(CP_UTF8, 0, text, -1, nullptr, 0, nullptr, nullptr);
  if (size <= 1) {
    return {};
  }
  std::string out(size - 1, '\0');
  WideCharToMultiByte(CP_UTF8, 0, text, -1, out.data(), size, nullptr,
                      nullptr);
  return out;
}

// Media Foundation needs starting once per process before an MFT is used;
// it's never shut down, like the encoders it serves
void StartMediaFoundation() {
  static std::once_flag once;
  std::call_once(once, [] { MFStartup(MF_VERSION, MFSTARTUP_LITE); });
}

// The thread's COM, in the multithreaded apartment Media Foundation expects.
// A thread already in another apartment keeps it: the MFTs are free threaded.
class ScopedCom {
 public:
  ScopedCom()
      : initialized_(SUCCEEDED(CoInitializeEx(nullptr, COINIT_MULTITHREADED))) {
  }
  ~ScopedCom() {
    if (initialized_) {
      CoUninitialize();
    }
  }

 private:
  const bool initialized_;
};

struct EncoderMft {
  ComPtr<IMFActivate> activate;
  std::string name;
};

// Hardware H.264 encoders taking NV12, in the order the system prefers them
std::vector<EncoderMft> FindEncoders() {
  MFT_REGISTER_TYPE_INFO input{MFMediaType_Video, MFVideoFormat_NV12};
  MFT_REGISTER_TYPE_INFO output{MFMediaType_Video, MFVideoFormat_H264};
  IMFActivate** list = nullptr;
  UINT32 count = 0;
  std::vector<EncoderMft> found;
  if (FAILED(MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER,
                       MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
                       &input, &output, &list, &count))) {
    return found;
  }
  for (UINT32 i = 0; i < count; ++i) {
    EncoderMft mft;
    mft.activate.Attach(list[i]);
    wchar_t* name = nullptr;
    UINT32 length = 0;
    if (SUCCEEDED(mft.activate->GetAllocatedString(MFT_FRIENDLY_NAME_Attribute,
                                                   &name, &length))) {
      mft.name = Utf8(name);
      CoTaskMemFree(name);
    }
    found.push_back(std::move(mft));
  }
  CoTaskMemFree(list);
  return found;
}

bool SetCodecValue(ICodecAPI* api, const GUID& property, UINT32 value) {
  VARIANT variant;
  VariantInit(&variant);
  variant.vt = VT_UI4;
  variant.ulVal = value;
  return SUCCEEDED(api->SetValue(&property, &variant));
}

bool SetCodecFlag(ICodecAPI* api, const GUID& property, bool value) {
  VARIANT variant;
  VariantInit(&variant);
  variant.vt = VT_BOOL;
  variant.boolVal = value ? VARIANT_TRUE : VARIANT_FALSE;
  return SUCCEEDED(api->SetValue(&property, &variant));
}

// What a frame carries besides its pixels, kept until its output comes back
struct PendingFrame {
  LONGLONG sample_time = 0;
  uint32_t rtp_timestamp = 0;
  int64_t ntp_time_ms = 0;
  int64_t render_time_ms = 0;
  webrtc::VideoRotation rotation = webrtc::kVideoRotation_0;
  std::optional<webrtc::ColorSpace> color_space;
};

class MediaFoundationH264Encoder : public webrtc::VideoEncoder {
 public:
  MediaFoundationH264Encoder() = default;
  ~MediaFoundationH264Encoder() override { Release(); }

  int32_t InitEncode(const webrtc::VideoCodec* codec,
                     const Settings& settings) override;
  int32_t RegisterEncodeCompleteCallback(
      webrtc::EncodedImageCallback* callback) override;
  int32_t Release() override;
  int32_t Encode(const webrtc::VideoFrame& frame,
                 const std::vector<webrtc::VideoFrameType>* frame_types)
      override;
  void SetRates(const RateControlParameters& parameters) override;
  EncoderInfo GetEncoderInfo() const override;

 private:
  // Sets up `mft` to encode at the current size, rate and bitrate
  bool Configure(const EncoderMft& mft);
  void Events();
  void TakeOutput();
  void Deliver(IMFSample* sample);
  void Shutdown();

  ComPtr<IMFTransform> transform_;
  ComPtr<IMFActivate> activate_;
  ComPtr<IMFMediaEventGenerator> events_;
  ComPtr<ICodecAPI> codec_api_;
  DWORD input_id_ = 0;
  DWORD output_id_ = 0;
  bool provides_samples_ = false;
  DWORD output_size_ = 0;
  std::string name_ = "MediaFoundation";

  int width_ = 0;
  int height_ = 0;
  uint32_t framerate_ = 30;
  uint32_t bitrate_bps_ = 0;
  bool sending_ = false;
  bool key_frame_pending_ = true;
  LONGLONG last_sample_time_ = -1;

  std::thread event_thread_;
  // Guards what the event thread shares with WebRTC's encoder thread
  std::mutex mutex_;
  webrtc::EncodedImageCallback* callback_ = nullptr;
  int needs_input_ = 0;
  std::deque<PendingFrame> pending_;
  bool failed_ = false;

  // Event thread only
  webrtc::H264BitstreamParser parser_;
  std::vector<uint8_t> sequence_header_;
};

int32_t MediaFoundationH264Encoder::InitEncode(
    const webrtc::VideoCodec* codec,
    const Settings& settings) {
  if (!codec || codec->codecType != webrtc::kVideoCodecH264 ||
      codec->width < 2 || codec->height < 2 || codec->maxFramerate == 0) {
    return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
  }
  Release();

  width_ = codec->width;
  height_ = codec->height;
  framerate_ = std::max<uint32_t>(1, codec->maxFramerate);
  bitrate_bps_ = std::max<uint32_t>(100'000, codec->startBitrate * 1000);
  sending_ = true;
  key_frame_pending_ = true;
  last_sample_time_ = -1;

  ScopedCom com;
  StartMediaFoundation();
  // Each encoder found is tried in turn: with two GPUs, the other one still
  // works when the first refuses this size
  for (const EncoderMft& mft : FindEncoders()) {
    if (Configure(mft)) {
      RTC_LOG(LS_INFO) << "Media Foundation H264 encoder " << name_ << ": "
                       << width_ << "x" << height_ << " @ " << framerate_
                       << " fps, " << bitrate_bps_ << " bps";
      event_thread_ = std::thread([this] { Events(); });
      return WEBRTC_VIDEO_CODEC_OK;
    }
    Shutdown();
  }
  RTC_LOG(LS_WARNING) << "No Media Foundation H264 encoder takes " << width_
                      << "x" << height_;
  return WEBRTC_VIDEO_CODEC_ERROR;
}

bool MediaFoundationH264Encoder::Configure(const EncoderMft& mft) {
  activate_ = mft.activate;
  name_ = mft.name.empty() ? "MediaFoundation" : mft.name;
  if (FAILED(activate_->ActivateObject(IID_PPV_ARGS(&transform_)))) {
    return false;
  }

  ComPtr<IMFAttributes> attributes;
  if (FAILED(transform_->GetAttributes(&attributes)) ||
      FAILED(attributes->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK, TRUE))) {
    return false;
  }
  attributes->SetUINT32(MF_LOW_LATENCY, TRUE);

  ComPtr<IMFShutdown> shutdown;
  // The event thread only ends when the MFT shuts down, so one that can't
  // isn't used
  if (FAILED(transform_.As(&events_)) || FAILED(transform_.As(&codec_api_)) ||
      FAILED(transform_.As(&shutdown))) {
    RTC_LOG(LS_WARNING) << name_ << " lacks an async MFT interface";
    return false;
  }

  HRESULT hr = transform_->GetStreamIDs(1, &input_id_, 1, &output_id_);
  if (hr == E_NOTIMPL) {
    input_id_ = 0;
    output_id_ = 0;
  } else if (FAILED(hr)) {
    return false;
  }

  // Rate control is set before the types, where some encoders read it
  ICodecAPI* api = codec_api_.Get();
  SetCodecValue(api, CODECAPI_AVEncCommonRateControlMode,
                eAVEncCommonRateControlMode_CBR);
  SetCodecValue(api, CODECAPI_AVEncCommonMeanBitRate, bitrate_bps_);
  SetCodecFlag(api, CODECAPI_AVLowLatencyMode, true);
  SetCodecValue(api, CODECAPI_AVEncMPVDefaultBPictureCount, 0);
  // Key frames only when WebRTC asks for one (a new receiver, or loss)
  SetCodecValue(api, CODECAPI_AVEncMPVGOPSize, framerate_ * 3600);

  auto make_type = [&](const GUID& subtype) {
    ComPtr<IMFMediaType> type;
    if (FAILED(MFCreateMediaType(&type))) {
      return type;
    }
    type->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
    type->SetGUID(MF_MT_SUBTYPE, subtype);
    MFSetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, width_, height_);
    MFSetAttributeRatio(type.Get(), MF_MT_FRAME_RATE, framerate_, 1);
    MFSetAttributeRatio(type.Get(), MF_MT_PIXEL_ASPECT_RATIO, 1, 1);
    type->SetUINT32(MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive);
    return type;
  };

  // The output type goes first, as encoders require. Constrained baseline is
  // what every receiver decodes; an older encoder may only know baseline,
  // whose streams it then keeps to the constrained subset anyway.
  bool output_set = false;
  for (UINT32 profile :
       {static_cast<UINT32>(eAVEncH264VProfile_ConstrainedBase),
        static_cast<UINT32>(eAVEncH264VProfile_Base)}) {
    ComPtr<IMFMediaType> output = make_type(MFVideoFormat_H264);
    if (!output) {
      return false;
    }
    output->SetUINT32(MF_MT_AVG_BITRATE, bitrate_bps_);
    output->SetUINT32(MF_MT_MPEG2_PROFILE, profile);
    if (SUCCEEDED(transform_->SetOutputType(output_id_, output.Get(), 0))) {
      output_set = true;
      break;
    }
  }
  if (!output_set) {
    RTC_LOG(LS_WARNING) << name_ << " refused the H264 output type";
    return false;
  }

  ComPtr<IMFMediaType> input = make_type(MFVideoFormat_NV12);
  if (!input || FAILED(transform_->SetInputType(input_id_, input.Get(), 0))) {
    RTC_LOG(LS_WARNING) << name_ << " refused NV12 input at " << width_ << "x"
                        << height_;
    return false;
  }

  MFT_OUTPUT_STREAM_INFO info{};
  if (FAILED(transform_->GetOutputStreamInfo(output_id_, &info))) {
    return false;
  }
  provides_samples_ =
      (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES |
                       MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
  output_size_ = std::max<DWORD>(info.cbSize, width_ * height_ * 3 / 2);

  if (FAILED(transform_->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)) ||
      FAILED(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
                                        0)) ||
      FAILED(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,
                                        0))) {
    return false;
  }
  return true;
}

void MediaFoundationH264Encoder::Events() {
  ScopedCom com;
  for (;;) {
    ComPtr<IMFMediaEvent> event;
    // Returns MF_E_SHUTDOWN once Release() shuts the MFT down
    if (FAILED(events_->GetEvent(0, &event))) {
      return;
    }
    MediaEventType type = MEUnknown;
    event->GetType(&type);
    if (type == METransformNeedInput) {
      std::lock_guard<std::mutex> lock(mutex_);
      ++needs_input_;
    } else if (type == METransformHaveOutput) {
      TakeOutput();
    } else if (type == MEError) {
      HRESULT status = S_OK;
      event->GetStatus(&status);
      RTC_LOG(LS_ERROR) << name_ << " failed: 0x" << std::hex << status;
      std::lock_guard<std::mutex> lock(mutex_);
      failed_ = true;
    }
  }
}

void MediaFoundationH264Encoder::TakeOutput() {
  MFT_OUTPUT_DATA_BUFFER output{};
  output.dwStreamID = output_id_;
  ComPtr<IMFSample> own;
  if (!provides_samples_) {
    ComPtr<IMFMediaBuffer> buffer;
    if (FAILED(MFCreateSample(&own)) ||
        FAILED(MFCreateMemoryBuffer(output_size_, &buffer)) ||
        FAILED(own->AddBuffer(buffer.Get()))) {
      return;
    }
    output.pSample = own.Get();
  }

  DWORD status = 0;
  const HRESULT hr = transform_->ProcessOutput(0, 1, &output, &status);
  if (output.pEvents) {
    output.pEvents->Release();
  }
  // A sample the MFT made is ours to release; our own is held by `own`
  ComPtr<IMFSample> sample;
  if (provides_samples_) {
    sample.Attach(output.pSample);
  } else {
    sample = own;
  }

  if (hr == MF_E_TRANSFORM_STREAM_CHANGE) {
    // The encoder settled its output format: take the one it offers
    ComPtr<IMFMediaType> type;
    if (SUCCEEDED(transform_->GetOutputAvailableType(output_id_, 0, &type))) {
      transform_->SetOutputType(output_id_, type.Get(), 0);
    }
    return;
  }
  if (FAILED(hr) || !sample) {
    RTC_LOG(LS_WARNING) << name_ << " ProcessOutput failed: 0x" << std::hex
                        << hr;
    return;
  }
  Deliver(sample.Get());
}

void MediaFoundationH264Encoder::Deliver(IMFSample* sample) {
  LONGLONG sample_time = 0;
  if (FAILED(sample->GetSampleTime(&sample_time))) {
    return;
  }
  ComPtr<IMFMediaBuffer> buffer;
  if (FAILED(sample->ConvertToContiguousBuffer(&buffer))) {
    return;
  }
  BYTE* data = nullptr;
  DWORD length = 0;
  if (FAILED(buffer->Lock(&data, nullptr, &length))) {
    return;
  }
  std::vector<uint8_t> bitstream(data, data + length);
  buffer->Unlock();
  if (bitstream.empty()) {
    return;
  }

  PendingFrame frame;
  webrtc::EncodedImageCallback* callback = nullptr;
  int width = 0;
  int height = 0;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    // Frames the encoder skipped come before this one: forget them
    while (!pending_.empty() && pending_.front().sample_time < sample_time) {
      pending_.pop_front();
    }
    if (pending_.empty() || pending_.front().sample_time != sample_time) {
      return;
    }
    frame = std::move(pending_.front());
    pending_.pop_front();
    callback = callback_;
    width = width_;
    height = height_;
  }
  if (!callback) {
    return;
  }

  bool key = false;
  bool has_sps = false;
  for (const auto& nalu : webrtc::H264::FindNaluIndices(bitstream)) {
    if (nalu.payload_start_offset >= bitstream.size()) {
      continue;
    }
    const auto type =
        webrtc::H264::ParseNaluType(bitstream[nalu.payload_start_offset]);
    key |= type == webrtc::H264::kIdr;
    has_sps |= type == webrtc::H264::kSps;
  }
  if (key && !has_sps) {
    // A receiver can only start from a key frame that brings its parameter
    // sets; an encoder that keeps them in the media type gets them put back
    if (sequence_header_.empty()) {
      ComPtr<IMFMediaType> type;
      UINT32 size = 0;
      if (SUCCEEDED(transform_->GetOutputCurrentType(output_id_, &type)) &&
          SUCCEEDED(type->GetBlobSize(MF_MT_MPEG_SEQUENCE_HEADER, &size)) &&
          size > 0) {
        sequence_header_.resize(size);
        if (FAILED(type->GetBlob(MF_MT_MPEG_SEQUENCE_HEADER,
                                 sequence_header_.data(), size, nullptr))) {
          sequence_header_.clear();
        }
      }
    }
    bitstream.insert(bitstream.begin(), sequence_header_.begin(),
                     sequence_header_.end());
  }

  webrtc::EncodedImage image;
  image.SetEncodedData(
      webrtc::EncodedImageBuffer::Create(bitstream.data(), bitstream.size()));
  image._encodedWidth = width;
  image._encodedHeight = height;
  image.SetRtpTimestamp(frame.rtp_timestamp);
  image.ntp_time_ms_ = frame.ntp_time_ms;
  image.capture_time_ms_ = frame.render_time_ms;
  image.rotation_ = frame.rotation;
  image.SetColorSpace(frame.color_space);
  image.content_type_ = webrtc::VideoContentType::UNSPECIFIED;
  image.timing_.flags = webrtc::VideoSendTiming::kInvalid;
  image._frameType = key ? webrtc::VideoFrameType::kVideoFrameKey
                         : webrtc::VideoFrameType::kVideoFrameDelta;
  parser_.ParseBitstream(image);
  image.qp_ = parser_.GetLastSliceQp().value_or(-1);

  webrtc::CodecSpecificInfo info;
  info.codecType = webrtc::kVideoCodecH264;
  info.codecSpecific.H264.packetization_mode =
      webrtc::H264PacketizationMode::NonInterleaved;
  callback->OnEncodedImage(image, &info);
}

int32_t MediaFoundationH264Encoder::RegisterEncodeCompleteCallback(
    webrtc::EncodedImageCallback* callback) {
  std::lock_guard<std::mutex> lock(mutex_);
  callback_ = callback;
  return WEBRTC_VIDEO_CODEC_OK;
}

void MediaFoundationH264Encoder::Shutdown() {
  if (transform_) {
    transform_->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
    ComPtr<IMFShutdown> shutdown;
    if (SUCCEEDED(transform_.As(&shutdown))) {
      shutdown->Shutdown();
    }
  }
  if (event_thread_.joinable()) {
    event_thread_.join();
  }
  if (activate_) {
    activate_->ShutdownObject();
  }
  codec_api_.Reset();
  events_.Reset();
  transform_.Reset();
  activate_.Reset();
  sequence_header_.clear();
  std::lock_guard<std::mutex> lock(mutex_);
  needs_input_ = 0;
  pending_.clear();
  failed_ = false;
}

int32_t MediaFoundationH264Encoder::Release() {
  Shutdown();
  return WEBRTC_VIDEO_CODEC_OK;
}

int32_t MediaFoundationH264Encoder::Encode(
    const webrtc::VideoFrame& frame,
    const std::vector<webrtc::VideoFrameType>* frame_types) {
  if (!transform_) {
    return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
  }
  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (!callback_) {
      return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
    }
    if (failed_) {
      // WebRTC moves to another encoder
      return WEBRTC_VIDEO_CODEC_ENCODER_FAILURE;
    }
  }
  if (!sending_) {
    return WEBRTC_VIDEO_CODEC_NO_OUTPUT;
  }
  bool key = key_frame_pending_;
  if (frame_types) {
    for (auto type : *frame_types) {
      if (type == webrtc::VideoFrameType::kEmptyFrame) {
        return WEBRTC_VIDEO_CODEC_NO_OUTPUT;
      }
      key |= type == webrtc::VideoFrameType::kVideoFrameKey;
    }
  }

  webrtc::scoped_refptr<webrtc::I420BufferInterface> i420 =
      frame.video_frame_buffer()->ToI420();
  if (!i420) {
    return WEBRTC_VIDEO_CODEC_ENCODER_FAILURE;
  }
  if (i420->width() != width_ || i420->height() != height_) {
    // WebRTC reinitializes on a new size, so this is a frame caught between
    auto scaled = webrtc::I420Buffer::Create(width_, height_);
    scaled->ScaleFrom(*i420);
    i420 = scaled;
  }

  {
    std::lock_guard<std::mutex> lock(mutex_);
    if (needs_input_ == 0) {
      // Still busy with earlier frames: drop this one rather than queue it,
      // keeping a key frame asked for to the next
      key_frame_pending_ = key;
      return WEBRTC_VIDEO_CODEC_OK;
    }
    --needs_input_;
  }

  const DWORD y_size = width_ * height_;
  const DWORD size = y_size + 2 * ((width_ + 1) / 2) * ((height_ + 1) / 2);
  ComPtr<IMFMediaBuffer> buffer;
  ComPtr<IMFSample> sample;
  BYTE* data = nullptr;
  if (FAILED(MFCreateMemoryBuffer(size, &buffer)) ||
      FAILED(buffer->Lock(&data, nullptr, nullptr))) {
    return WEBRTC_VIDEO_CODEC_ENCODER_FAILURE;
  }
  libyuv::I420ToNV12(i420->DataY(), i420->StrideY(), i420->DataU(),
                     i420->StrideU(), i420->DataV(), i420->StrideV(), data,
                     width_, data + y_size, width_ + (width_ & 1), width_,
                     height_);
  buffer->Unlock();
  buffer->SetCurrentLength(size);
  if (FAILED(MFCreateSample(&sample)) ||
      FAILED(sample->AddBuffer(buffer.Get()))) {
    return WEBRTC_VIDEO_CODEC_ENCODER_FAILURE;
  }

  // In 100 ns units, and always rising, since the output is matched to its
  // frame by it
  LONGLONG sample_time = frame.timestamp_us() * 10;
  if (sample_time <= last_sample_time_) {
    sample_time = last_sample_time_ + 1;
  }
  last_sample_time_ = sample_time;
  sample->SetSampleTime(sample_time);
  sample->SetSampleDuration(10'000'000 / framerate_);

  if (key) {
    SetCodecValue(codec_api_.Get(), CODECAPI_AVEncVideoForceKeyFrame, 1);
    key_frame_pending_ = false;
  }

  {
    std::lock_guard<std::mutex> lock(mutex_);
    PendingFrame pending;
    pending.sample_time = sample_time;
    pending.rtp_timestamp = frame.rtp_timestamp();
    pending.ntp_time_ms = frame.ntp_time_ms();
    pending.render_time_ms = frame.render_time_ms();
    pending.rotation = frame.rotation();
    pending.color_space = frame.color_space();
    pending_.push_back(std::move(pending));
    while (pending_.size() > kMaxPending) {
      pending_.pop_front();
    }
  }

  const HRESULT hr = transform_->ProcessInput(input_id_, sample.Get(), 0);
  if (FAILED(hr)) {
    RTC_LOG(LS_ERROR) << name_ << " ProcessInput failed: 0x" << std::hex << hr;
    std::lock_guard<std::mutex> lock(mutex_);
    if (!pending_.empty()) {
      pending_.pop_back();
    }
    return WEBRTC_VIDEO_CODEC_ENCODER_FAILURE;
  }
  return WEBRTC_VIDEO_CODEC_OK;
}

void MediaFoundationH264Encoder::SetRates(
    const RateControlParameters& parameters) {
  if (!transform_) {
    return;
  }
  const uint32_t bitrate = parameters.bitrate.get_sum_bps();
  if (bitrate == 0) {
    sending_ = false;
    return;
  }
  if (!sending_) {
    // The receiver may have lost the stream while it was paused
    key_frame_pending_ = true;
  }
  sending_ = true;
  if (std::isfinite(parameters.framerate_fps) &&
      parameters.framerate_fps >= 1.0) {
    framerate_ = static_cast<uint32_t>(std::round(parameters.framerate_fps));
  }
  if (bitrate != bitrate_bps_) {
    bitrate_bps_ = bitrate;
    SetCodecValue(codec_api_.Get(), CODECAPI_AVEncCommonMeanBitRate, bitrate);
  }
}

webrtc::VideoEncoder::EncoderInfo MediaFoundationH264Encoder::GetEncoderInfo()
    const {
  EncoderInfo info;
  info.implementation_name = name_;
  info.is_hardware_accelerated = true;
  info.supports_native_handle = false;
  info.supports_simulcast = false;
  info.scaling_settings = ScalingSettings(kLowQp, kHighQp);
  info.requested_resolution_alignment = 2;
  info.preferred_pixel_formats = {webrtc::VideoFrameBuffer::Type::kI420};
  return info;
}

class MediaFoundationEncoderFactory : public webrtc::VideoEncoderFactory {
 public:
  std::vector<webrtc::SdpVideoFormat> GetSupportedFormats() const override {
    return {
        webrtc::CreateH264Format(webrtc::H264Profile::kProfileConstrainedBaseline,
                                 webrtc::H264Level::kLevel3_1, "1"),
        webrtc::CreateH264Format(webrtc::H264Profile::kProfileBaseline,
                                 webrtc::H264Level::kLevel3_1, "1"),
    };
  }

  std::unique_ptr<webrtc::VideoEncoder> Create(
      const webrtc::Environment& env,
      const webrtc::SdpVideoFormat& format) override {
    for (const auto& supported : GetSupportedFormats()) {
      if (format.IsSameCodec(supported)) {
        return std::make_unique<MediaFoundationH264Encoder>();
      }
    }
    return nullptr;
  }
};

}  // namespace

bool MediaFoundationH264Available() {
  static const bool available = [] {
    // On a thread of its own, so the caller's COM apartment doesn't matter
    bool found = false;
    std::thread([&found] {
      ScopedCom com;
      StartMediaFoundation();
      for (const EncoderMft& mft : FindEncoders()) {
        RTC_LOG(LS_INFO) << "Media Foundation H264 encoder found: "
                         << mft.name;
        found = true;
      }
    }).join();
    return found;
  }();
  return available;
}

std::unique_ptr<webrtc::VideoEncoderFactory>
CreateMediaFoundationEncoderFactory() {
  return std::make_unique<MediaFoundationEncoderFactory>();
}

}  // namespace livekit_ffi

#endif  // defined(_WIN32)
