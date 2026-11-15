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

#include "livekit/flexfec.h"

#include <atomic>
#include <string>

#include "webrtc-sys/src/flexfec.rs.h"

namespace livekit_ffi {

namespace {

std::atomic<bool> g_flexfec{false};

class FlexFecFieldTrials final : public webrtc::FieldTrialsView {
 public:
  std::string Lookup(absl::string_view key) const override {
    if (g_flexfec.load() && (key == "WebRTC-FlexFEC-03" ||
                             key == "WebRTC-FlexFEC-03-Advertised")) {
      return "Enabled";
    }
    return "";
  }

  std::unique_ptr<webrtc::FieldTrialsView> CreateCopy() const override {
    return std::make_unique<FlexFecFieldTrials>();
  }
};

}  // namespace

std::unique_ptr<webrtc::FieldTrialsView> create_flexfec_field_trials() {
  return std::make_unique<FlexFecFieldTrials>();
}

void set_flexfec(bool enabled) {
  g_flexfec.store(enabled);
}

}  // namespace livekit_ffi
