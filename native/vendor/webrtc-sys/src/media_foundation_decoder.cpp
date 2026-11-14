// Hardware H.264 decoding through Media Foundation. WebRTC on Windows decodes
// H.264 with FFmpeg on the CPU, which for a 60 fps shared screen costs more
// than the encoding on the other side.
//
// Microsoft's H.264 decoder MFT decodes on the GPU (DXVA) once it's handed a
// D3D11 device; without one it decodes in software too, so it's only used with
// one. It's synchronous: each Decode() feeds one frame and takes whatever
// pictures are ready. The pictures come back as GPU textures and are read back
// as NV12, then converted to the I420 the rest of WebRTC expects. Anything
// failing hands the stream to FFmpeg through WebRTC's fallback wrapper.

#if defined(_WIN32)

#include "livekit/media_foundation_decoder.h"

#include <algorithm>
#include <deque>
#include <ios>
#include <mutex>
#include <optional>
#include <string>
#include <thread>

#include "api/video/color_space.h"
#include "api/video/encoded_image.h"
#include "api/video/i420_buffer.h"
#include "api/video/video_frame.h"
#include "api/video_codecs/video_decoder.h"
#include "common_video/include/video_frame_buffer_pool.h"
#include "modules/video_coding/include/video_error_codes.h"
#include "rtc_base/logging.h"
#include "third_party/libyuv/include/libyuv/convert.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>

#include <codecapi.h>
#include <d3d11.h>
// After d3d11.h, which pulls in parts of it
#include <d3d10_1.h>
#include <mfapi.h>
#include <mferror.h>
#include <mfidl.h>
#include <mftransform.h>
#include <wmcodecdsp.h>
#include <wrl/client.h>

namespace livekit_ffi {

namespace {

using Microsoft::WRL::ComPtr;

// Frames fed in without a picture back yet. The decoder holds none in low
// latency mode; past this something is stuck, and the oldest are forgotten.
constexpr size_t kMaxPending = 30;
// Decoded pictures held by WebRTC and the renderer at once; past this a picture
// is skipped rather than allocated
constexpr size_t kBufferPoolSize = 32;

void StartMediaFoundation() {
  static std::once_flag once;
  std::call_once(once, [] { MFStartup(MF_VERSION, MFSTARTUP_LITE); });
}

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

// What a frame carries besides its bitstream, kept until its picture comes
struct PendingFrame {
  LONGLONG sample_time = 0;
  uint32_t rtp_timestamp = 0;
  int64_t ntp_time_ms = 0;
  webrtc::VideoRotation rotation = webrtc::kVideoRotation_0;
  std::optional<webrtc::ColorSpace> color_space;
};

class MediaFoundationH264Decoder : public webrtc::VideoDecoder {
 public:
  MediaFoundationH264Decoder() : pool_(false, kBufferPoolSize) {}
  ~MediaFoundationH264Decoder() override { Release(); }

  bool Configure(const Settings& settings) override;
  int32_t Decode(const webrtc::EncodedImage& input,
                 bool missing_frames,
                 int64_t render_time_ms) override;
  int32_t RegisterDecodeCompleteCallback(
      webrtc::DecodedImageCallback* callback) override;
  int32_t Release() override;
  DecoderInfo GetDecoderInfo() const override;
  const char* ImplementationName() const override {
    return "MediaFoundation";
  }

 private:
  bool CreateDevice();
  // Picks NV12 out of what the decoder offers, and reads the picture size
  bool SetOutputType();
  // False when the decoder failed; no picture ready isn't a failure
  bool TakeOutput();
  void Deliver(IMFSample* sample);

  ComPtr<ID3D11Device> device_;
  ComPtr<IMFDXGIDeviceManager> device_manager_;
  UINT reset_token_ = 0;
  ComPtr<IMFTransform> transform_;
  bool provides_samples_ = false;
  DWORD output_size_ = 0;

  // The coded size (macroblock aligned) and the picture shown from it
  UINT32 coded_width_ = 0;
  UINT32 coded_height_ = 0;
  int width_ = 0;
  int height_ = 0;
  LONG stride_ = 0;

  webrtc::DecodedImageCallback* callback_ = nullptr;
  webrtc::VideoFrameBufferPool pool_;
  std::deque<PendingFrame> pending_;
  LONGLONG last_sample_time_ = -1;
};

bool MediaFoundationH264Decoder::CreateDevice() {
  // Video support is what DXVA decoding runs on
  const D3D_FEATURE_LEVEL levels[] = {D3D_FEATURE_LEVEL_11_1,
                                      D3D_FEATURE_LEVEL_11_0,
                                      D3D_FEATURE_LEVEL_10_1};
  if (FAILED(D3D11CreateDevice(
          nullptr, D3D_DRIVER_TYPE_HARDWARE, nullptr,
          D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
          levels, ARRAYSIZE(levels), D3D11_SDK_VERSION, &device_, nullptr,
          nullptr))) {
    return false;
  }
  // The decoder and our readback reach the device from different threads
  ComPtr<ID3D10Multithread> multithread;
  if (SUCCEEDED(device_.As(&multithread))) {
    multithread->SetMultithreadProtected(TRUE);
  }
  return SUCCEEDED(
             MFCreateDXGIDeviceManager(&reset_token_, &device_manager_)) &&
         SUCCEEDED(device_manager_->ResetDevice(device_.Get(), reset_token_));
}

bool MediaFoundationH264Decoder::Configure(const Settings& settings) {
  if (settings.codec_type() != webrtc::kVideoCodecH264) {
    return false;
  }
  Release();

  ScopedCom com;
  StartMediaFoundation();
  if (FAILED(CoCreateInstance(CLSID_CMSH264DecoderMFT, nullptr,
                              CLSCTX_INPROC_SERVER,
                              IID_PPV_ARGS(&transform_)))) {
    RTC_LOG(LS_WARNING) << "No Media Foundation H264 decoder";
    return false;
  }

  ComPtr<IMFAttributes> attributes;
  UINT32 d3d11_aware = FALSE;
  if (FAILED(transform_->GetAttributes(&attributes)) ||
      FAILED(attributes->GetUINT32(MF_SA_D3D11_AWARE, &d3d11_aware)) ||
      !d3d11_aware || !CreateDevice() ||
      FAILED(transform_->ProcessMessage(
          MFT_MESSAGE_SET_D3D_MANAGER,
          reinterpret_cast<ULONG_PTR>(device_manager_.Get())))) {
    RTC_LOG(LS_WARNING) << "Media Foundation H264 decoder can't use the GPU";
    Release();
    return false;
  }
  // Each frame comes out as soon as it's decoded, without reordering delay:
  // WebRTC's H.264 has no B-frames
  attributes->SetUINT32(CODECAPI_AVLowLatencyMode, TRUE);
  ComPtr<ICodecAPI> codec_api;
  if (SUCCEEDED(transform_.As(&codec_api))) {
    VARIANT on;
    VariantInit(&on);
    on.vt = VT_UI4;
    on.ulVal = 1;
    codec_api->SetValue(&CODECAPI_AVLowLatencyMode, &on);
  }

  ComPtr<IMFMediaType> input;
  if (FAILED(MFCreateMediaType(&input)) ||
      FAILED(input->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video)) ||
      FAILED(input->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_H264)) ||
      FAILED(input->SetUINT32(MF_MT_INTERLACE_MODE,
                              MFVideoInterlace_Progressive)) ||
      FAILED(transform_->SetInputType(0, input.Get(), 0)) ||
      !SetOutputType() ||
      FAILED(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
                                        0)) ||
      FAILED(transform_->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,
                                        0))) {
    RTC_LOG(LS_WARNING) << "Media Foundation H264 decoder refused its types";
    Release();
    return false;
  }
  RTC_LOG(LS_INFO) << "Media Foundation H264 decoder on the GPU";
  return true;
}

bool MediaFoundationH264Decoder::SetOutputType() {
  for (DWORD i = 0;; ++i) {
    ComPtr<IMFMediaType> type;
    if (FAILED(transform_->GetOutputAvailableType(0, i, &type))) {
      return false;
    }
    GUID subtype{};
    if (FAILED(type->GetGUID(MF_MT_SUBTYPE, &subtype)) ||
        subtype != MFVideoFormat_NV12) {
      continue;
    }
    if (FAILED(transform_->SetOutputType(0, type.Get(), 0))) {
      return false;
    }
    // Before the first key frame the size is a placeholder; the stream change
    // that follows it brings the real one
    MFGetAttributeSize(type.Get(), MF_MT_FRAME_SIZE, &coded_width_,
                       &coded_height_);
    width_ = static_cast<int>(coded_width_);
    height_ = static_cast<int>(coded_height_);
    MFVideoArea area{};
    if (SUCCEEDED(type->GetBlob(MF_MT_MINIMUM_DISPLAY_APERTURE,
                                reinterpret_cast<UINT8*>(&area), sizeof(area),
                                nullptr))) {
      width_ = std::min<int>(width_, area.Area.cx);
      height_ = std::min<int>(height_, area.Area.cy);
    }
    UINT32 stride = 0;
    stride_ = SUCCEEDED(type->GetUINT32(MF_MT_DEFAULT_STRIDE, &stride))
                  ? static_cast<LONG>(stride)
                  : static_cast<LONG>(coded_width_);

    MFT_OUTPUT_STREAM_INFO info{};
    if (FAILED(transform_->GetOutputStreamInfo(0, &info))) {
      return false;
    }
    provides_samples_ =
        (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES |
                         MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES)) != 0;
    output_size_ = std::max<DWORD>(info.cbSize,
                                   coded_width_ * coded_height_ * 3 / 2);
    return true;
  }
}

int32_t MediaFoundationH264Decoder::Decode(const webrtc::EncodedImage& input,
                                           bool /*missing_frames*/,
                                           int64_t /*render_time_ms*/) {
  if (!transform_ || !callback_) {
    return WEBRTC_VIDEO_CODEC_UNINITIALIZED;
  }
  if (!input.data() || input.size() == 0) {
    return WEBRTC_VIDEO_CODEC_ERR_PARAMETER;
  }

  ComPtr<IMFMediaBuffer> buffer;
  ComPtr<IMFSample> sample;
  BYTE* data = nullptr;
  const DWORD size = static_cast<DWORD>(input.size());
  if (FAILED(MFCreateMemoryBuffer(size, &buffer)) ||
      FAILED(buffer->Lock(&data, nullptr, nullptr))) {
    return WEBRTC_VIDEO_CODEC_ERROR;
  }
  memcpy(data, input.data(), size);
  buffer->Unlock();
  buffer->SetCurrentLength(size);
  if (FAILED(MFCreateSample(&sample)) ||
      FAILED(sample->AddBuffer(buffer.Get()))) {
    return WEBRTC_VIDEO_CODEC_ERROR;
  }

  // The picture is matched to its frame by this, so it always rises; the RTP
  // timestamp wraps
  LONGLONG sample_time = last_sample_time_ + 1;
  last_sample_time_ = sample_time;
  sample->SetSampleTime(sample_time);

  PendingFrame pending;
  pending.sample_time = sample_time;
  pending.rtp_timestamp = input.RtpTimestamp();
  pending.ntp_time_ms = input.ntp_time_ms_;
  pending.rotation = input.rotation_;
  if (const webrtc::ColorSpace* color_space = input.ColorSpace()) {
    pending.color_space = *color_space;
  }
  pending_.push_back(std::move(pending));
  while (pending_.size() > kMaxPending) {
    pending_.pop_front();
  }

  HRESULT hr = transform_->ProcessInput(0, sample.Get(), 0);
  if (hr == MF_E_NOTACCEPTING) {
    // A picture still waits: take it, then this frame fits
    if (!TakeOutput()) {
      return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
    }
    hr = transform_->ProcessInput(0, sample.Get(), 0);
  }
  if (FAILED(hr)) {
    RTC_LOG(LS_WARNING) << "Media Foundation H264 ProcessInput failed: 0x"
                        << std::hex << hr;
    pending_.pop_back();
    return WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
  }
  return TakeOutput() ? WEBRTC_VIDEO_CODEC_OK
                      : WEBRTC_VIDEO_CODEC_FALLBACK_SOFTWARE;
}

bool MediaFoundationH264Decoder::TakeOutput() {
  for (;;) {
    MFT_OUTPUT_DATA_BUFFER output{};
    ComPtr<IMFSample> own;
    if (!provides_samples_) {
      ComPtr<IMFMediaBuffer> buffer;
      if (FAILED(MFCreateSample(&own)) ||
          FAILED(MFCreateMemoryBuffer(output_size_, &buffer)) ||
          FAILED(own->AddBuffer(buffer.Get()))) {
        return false;
      }
      output.pSample = own.Get();
    }

    DWORD status = 0;
    const HRESULT hr = transform_->ProcessOutput(0, 1, &output, &status);
    if (output.pEvents) {
      output.pEvents->Release();
    }
    ComPtr<IMFSample> sample;
    if (provides_samples_) {
      sample.Attach(output.pSample);
    } else {
      sample = own;
    }

    if (hr == MF_E_TRANSFORM_NEED_MORE_INPUT) {
      return true;
    }
    if (hr == MF_E_TRANSFORM_STREAM_CHANGE) {
      // The first key frame, or a new size: the picture's format is known now
      if (!SetOutputType()) {
        return false;
      }
      continue;
    }
    if (FAILED(hr)) {
      RTC_LOG(LS_WARNING) << "Media Foundation H264 ProcessOutput failed: 0x"
                          << std::hex << hr;
      return false;
    }
    if (sample) {
      Deliver(sample.Get());
    }
  }
}

void MediaFoundationH264Decoder::Deliver(IMFSample* sample) {
  LONGLONG sample_time = 0;
  if (FAILED(sample->GetSampleTime(&sample_time))) {
    return;
  }
  // Frames the decoder dropped (a broken reference) come before this one
  while (!pending_.empty() && pending_.front().sample_time < sample_time) {
    pending_.pop_front();
  }
  if (pending_.empty() || pending_.front().sample_time != sample_time) {
    return;
  }
  PendingFrame frame = std::move(pending_.front());
  pending_.pop_front();

  ComPtr<IMFMediaBuffer> buffer;
  if (FAILED(sample->GetBufferByIndex(0, &buffer))) {
    return;
  }
  webrtc::scoped_refptr<webrtc::I420Buffer> i420 =
      pool_.CreateI420Buffer(width_, height_);
  if (!i420) {
    // Every buffer is still with the renderer: skip this picture
    return;
  }

  // A GPU texture is read back by the lock; a 2D buffer gives its own pitch
  ComPtr<IMF2DBuffer> buffer_2d;
  BYTE* data = nullptr;
  LONG pitch = 0;
  bool locked_2d = false;
  if (SUCCEEDED(buffer.As(&buffer_2d)) &&
      SUCCEEDED(buffer_2d->Lock2D(&data, &pitch))) {
    locked_2d = true;
  } else if (SUCCEEDED(buffer->Lock(&data, nullptr, nullptr))) {
    pitch = stride_;
  } else {
    return;
  }
  // NV12's chroma follows the luma's coded height, not the shown one
  const BYTE* uv = data + static_cast<size_t>(pitch) * coded_height_;
  libyuv::NV12ToI420(data, pitch, uv, pitch, i420->MutableDataY(),
                     i420->StrideY(), i420->MutableDataU(), i420->StrideU(),
                     i420->MutableDataV(), i420->StrideV(), width_, height_);
  if (locked_2d) {
    buffer_2d->Unlock2D();
  } else {
    buffer->Unlock();
  }

  webrtc::VideoFrame decoded = webrtc::VideoFrame::Builder()
                                   .set_video_frame_buffer(i420)
                                   .set_rtp_timestamp(frame.rtp_timestamp)
                                   .set_ntp_time_ms(frame.ntp_time_ms)
                                   .set_rotation(frame.rotation)
                                   .set_color_space(frame.color_space)
                                   .build();
  callback_->Decoded(decoded, std::nullopt, std::nullopt);
}

int32_t MediaFoundationH264Decoder::RegisterDecodeCompleteCallback(
    webrtc::DecodedImageCallback* callback) {
  callback_ = callback;
  return WEBRTC_VIDEO_CODEC_OK;
}

int32_t MediaFoundationH264Decoder::Release() {
  if (transform_) {
    transform_->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
    transform_->ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0);
  }
  transform_.Reset();
  device_manager_.Reset();
  device_.Reset();
  pending_.clear();
  pool_.Release();
  last_sample_time_ = -1;
  return WEBRTC_VIDEO_CODEC_OK;
}

webrtc::VideoDecoder::DecoderInfo MediaFoundationH264Decoder::GetDecoderInfo()
    const {
  DecoderInfo info;
  info.implementation_name = "MediaFoundation";
  info.is_hardware_accelerated = true;
  return info;
}

}  // namespace

bool MediaFoundationH264DecoderAvailable() {
  static const bool available = [] {
    // On a thread of its own, so the caller's COM apartment doesn't matter
    bool found = false;
    std::thread([&found] {
      ScopedCom com;
      StartMediaFoundation();
      MFT_REGISTER_TYPE_INFO input{MFMediaType_Video, MFVideoFormat_H264};
      IMFActivate** list = nullptr;
      UINT32 count = 0;
      // The GPU's decoding is reached through Microsoft's MFT, not one of its
      // own, so this asks whether the system has any H.264 decoder at all;
      // Configure() then checks for the GPU
      if (SUCCEEDED(MFTEnumEx(MFT_CATEGORY_VIDEO_DECODER,
                              MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
                              &input, nullptr, &list, &count))) {
        found = count > 0;
        for (UINT32 i = 0; i < count; ++i) {
          list[i]->Release();
        }
        CoTaskMemFree(list);
      }
    }).join();
    return found;
  }();
  return available;
}

std::unique_ptr<webrtc::VideoDecoder> CreateMediaFoundationH264Decoder() {
  return std::make_unique<MediaFoundationH264Decoder>();
}

}  // namespace livekit_ffi

#endif  // defined(_WIN32)
