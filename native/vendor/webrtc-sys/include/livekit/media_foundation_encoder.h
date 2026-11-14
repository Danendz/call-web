#pragma once

#if defined(_WIN32)

#include <memory>

#include "api/video_codecs/video_encoder_factory.h"

namespace livekit_ffi {

// Whether the machine has a hardware H.264 encoder that Media Foundation can
// reach: the GPU vendor's encoder MFT (NVIDIA, AMD or Intel). Checked once.
bool MediaFoundationH264Available();

// H.264 encoded by that hardware MFT, constrained baseline in packetization
// mode 1.
std::unique_ptr<webrtc::VideoEncoderFactory>
CreateMediaFoundationEncoderFactory();

}  // namespace livekit_ffi

#endif
