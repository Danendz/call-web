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

#include "api/field_trials_view.h"

namespace livekit_ffi {

// Field trials that turn FlexFEC on for video while set_flexfec(true): the
// factory always carries this view, and it reads the switch on each lookup, so
// a peer connection made after a switch gets it.
std::unique_ptr<webrtc::FieldTrialsView> create_flexfec_field_trials();

// Whether video peer connections made from now on offer and send FlexFEC.
void set_flexfec(bool enabled);

}  // namespace livekit_ffi
