// Copyright 2025 LiveKit, Inc.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! A switch for FlexFEC on video: forward error correction that lets the
//! receiver rebuild lost packets without waiting a round trip for them.

#[cxx::bridge(namespace = "livekit_ffi")]
mod ffi {
    unsafe extern "C++" {
        include!("livekit/flexfec.h");

        fn set_flexfec(enabled: bool);
    }
}

/// Turns FlexFEC on or off for peer connections made from now on; one already
/// connected keeps what it negotiated. Both sides need it on for it to be used.
pub fn set_flexfec(enabled: bool) {
    ffi::set_flexfec(enabled);
}
