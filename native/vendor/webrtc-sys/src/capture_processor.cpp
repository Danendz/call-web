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

#include "livekit/capture_processor.h"

#include <algorithm>
#include <atomic>
#include <mutex>
#include <string>

#include "api/audio/builtin_audio_processing_builder.h"
#include "modules/audio_processing/audio_buffer.h"
#include "rust/cxx.h"
#include "webrtc-sys/src/capture_processor.rs.h"

namespace livekit_ffi {

namespace {

std::atomic<bool> g_builtin_ns{true};
std::mutex g_mutex;
// The latest ones built; a factory is made once per app in practice
webrtc::scoped_refptr<webrtc::AudioProcessing> g_apm;
webrtc::scoped_refptr<webrtc::AudioSourceInterface> g_device_source;

class RustCaptureProcessor : public webrtc::CustomProcessing {
 public:
  void Initialize(int sample_rate_hz, int num_channels) override {
    sample_rate_hz_ = sample_rate_hz;
  }

  // Samples are floats in the 16-bit range. The first channel is processed and
  // copied to the others: a mic is mono, and one model state can't serve two.
  void Process(webrtc::AudioBuffer* audio) override {
    if (audio->num_channels() == 0 || audio->num_frames() == 0)
      return;
    float* const* channels = audio->channels();
    process_capture(rust::Slice<float>(channels[0], audio->num_frames()),
                    sample_rate_hz_);
    for (size_t ch = 1; ch < audio->num_channels(); ++ch)
      std::copy(channels[0], channels[0] + audio->num_frames(), channels[ch]);
  }

  std::string ToString() const override { return "RustCaptureProcessor"; }

 private:
  int sample_rate_hz_ = 0;
};

class KeepingBuilder : public webrtc::AudioProcessingBuilderInterface {
 public:
  webrtc::scoped_refptr<webrtc::AudioProcessing> Build(
      const webrtc::Environment& env) override {
    auto apm = webrtc::BuiltinAudioProcessingBuilder()
                   .SetCapturePostProcessing(
                       std::make_unique<RustCaptureProcessor>())
                   .Build(env);
    std::lock_guard<std::mutex> lock(g_mutex);
    g_apm = apm;
    return apm;
  }
};

}  // namespace

std::unique_ptr<webrtc::AudioProcessingBuilderInterface>
create_capture_processing_builder() {
  return std::make_unique<KeepingBuilder>();
}

void track_device_audio_source(
    webrtc::scoped_refptr<webrtc::AudioSourceInterface> source) {
  std::lock_guard<std::mutex> lock(g_mutex);
  g_device_source = source;
}

bool builtin_noise_suppression() {
  return g_builtin_ns.load();
}

void set_builtin_noise_suppression(bool enabled) {
  g_builtin_ns.store(enabled);
  std::lock_guard<std::mutex> lock(g_mutex);
  // The source's options are applied again whenever its sender changes (a
  // mute, a renegotiation), so they have to agree with the live setting
  if (g_device_source) {
    webrtc::AudioOptions options = g_device_source->options();
    options.noise_suppression = enabled;
    g_device_source->SetOptions(options);
  }
  if (g_apm) {
    webrtc::AudioProcessing::Config config = g_apm->GetConfig();
    config.noise_suppression.enabled = enabled;
    g_apm->ApplyConfig(config);
  }
}

}  // namespace livekit_ffi
