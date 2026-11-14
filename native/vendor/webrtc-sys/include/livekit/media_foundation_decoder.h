#pragma once

#if defined(_WIN32)

#include <memory>

#include "api/video_codecs/video_decoder.h"

namespace livekit_ffi {

// Whether Media Foundation has an H.264 decoder. Checked once.
bool MediaFoundationH264DecoderAvailable();

// H.264 decoded on the GPU through Media Foundation. Configure() fails where
// the GPU can't be used, so it's meant to sit in front of a software decoder.
std::unique_ptr<webrtc::VideoDecoder> CreateMediaFoundationH264Decoder();

}  // namespace livekit_ffi

#endif
