/*
 * Copyright 2025 LiveKit, Inc.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

#pragma once

#include <memory>

#include "api/audio/audio_processing.h"
#include "api/media_stream_interface.h"
#include "api/scoped_refptr.h"

namespace livekit_ffi {

// The audio processing builder for the peer connection factory: it adds a
// capture post-processor that hands every 10 ms mic frame (after echo
// cancellation and gain control) to the Rust callback set with
// `set_capture_processor`, and remembers the processing module it built so the
// built-in noise suppression can be switched while it runs.
std::unique_ptr<webrtc::AudioProcessingBuilderInterface>
create_capture_processing_builder();

// Applies the current built-in noise suppression setting to a new device
// (microphone) audio source, and remembers it so a later switch reaches it.
void track_device_audio_source(
    webrtc::scoped_refptr<webrtc::AudioSourceInterface> source);

// Whether a device audio source gets the built-in noise suppression.
bool builtin_noise_suppression();

// Turns WebRTC's own noise suppression on or off for the microphone, live.
void set_builtin_noise_suppression(bool enabled);

}  // namespace livekit_ffi
