//! Which video codec a call uses, like the web's `codec.ts`. In "Auto", H.264 goes first where this device encodes it
//! in hardware (VideoToolbox on macOS, MediaCodec on Android), because encoding VP8 in software is most of a call's
//! CPU. Without hardware H.264 is left out, so this side never runs a software H.264 encoder. "VP8" leaves out
//! everything else, which moves the whole call to VP8: the way out when H.264 misbehaves.
//!
//! WebRTC sends with the first codec the other side's description lists, and our own codec order only says what we'd
//! like to receive. So a codec we leave out is also taken out of the other side's offer before it's applied
//! ([`filter_offer`]); otherwise we'd still send it whenever they offer it first.

use libwebrtc::MediaType;
use libwebrtc::media_stream_track::MediaStreamTrack;
use libwebrtc::peer_connection::PeerConnection;
use libwebrtc::peer_connection_factory::PeerConnectionFactory;
use libwebrtc::rtp_parameters::RtpCodecCapability;
use libwebrtc::rtp_sender::VideoEncoderBackend;
use libwebrtc::video_codec::VideoCodec as Codec;

/// The "Video codec" picker in Settings.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VideoCodec {
    #[default]
    Auto,
    /// H.264 first even without a hardware encoder, VP8 kept in case the other side can't do H.264
    H264,
    /// VP8 only
    Vp8,
}

/// Whether this device encodes H.264 in hardware. A software OpenH264 doesn't count: it costs more than VP8.
pub(crate) fn hardware_h264(factory: &PeerConnectionFactory) -> bool {
    factory.video_encoder_codecs(VideoEncoderBackend::Hardware).contains(&Codec::H264)
}

/// Sets the codec order on every video transceiver, the other side's too: an answer follows our order only for
/// transceivers that have one. Takes effect at the next negotiation.
pub(crate) fn apply(factory: &PeerConnectionFactory, pc: &PeerConnection, pick: VideoCodec, hardware: bool) {
    let codecs = order(factory.get_rtp_receiver_capabilities(MediaType::Video).codecs, pick, hardware);
    for transceiver in pc.transceivers() {
        if !matches!(transceiver.receiver().track(), Some(MediaStreamTrack::Video(_))) {
            continue;
        }
        if let Err(e) = transceiver.set_codec_preferences(codecs.clone()) {
            tracing::warn!("couldn't set the video codecs: {}", e.message);
        }
    }
}

/// Whether a codec, by its lowercase name, may be used at all.
fn allowed(name: &str, pick: VideoCodec, hardware: bool) -> bool {
    match name {
        "h264" => pick == VideoCodec::H264 || (pick == VideoCodec::Auto && hardware),
        "vp8" => true,
        // Retransmission and error correction, which go with whatever codec is picked
        "rtx" | "red" | "ulpfec" | "flexfec-03" => true,
        _ => pick != VideoCodec::Vp8,
    }
}

fn order(mut codecs: Vec<RtpCodecCapability>, pick: VideoCodec, hardware: bool) -> Vec<RtpCodecCapability> {
    codecs.retain(|c| allowed(&name(c), pick, hardware));
    let fmtp = |c: &RtpCodecCapability, param| c.sdp_fmtp_line.as_deref().is_some_and(|l| fmtp_has(l, param));
    // A stable sort keeps the rest in WebRTC's own order
    codecs.sort_by_key(|c| match name(c).as_str() {
        // Constrained baseline in packetization mode 1: what every hardware encoder and browser can do. Its level
        // varies (WebRTC on a Mac lists 42e034, browsers 42e01f); level-asymmetry-allowed makes that not matter
        "h264" if fmtp(c, "packetization-mode=1") && fmtp(c, "profile-level-id=42e0") => 0,
        "h264" if fmtp(c, "packetization-mode=1") => 1,
        "h264" => 2,
        "vp8" => 3,
        _ => 4,
    });
    codecs
}

/// The other side's offer without the video codecs we leave out, and the retransmission entries that went with
/// them. A video section that would be left with no codec at all is kept whole: a codec we'd rather not use beats
/// no video. Nothing else changes, so their order still says what they'd like to receive.
pub(crate) fn filter_offer(sdp: &str, pick: VideoCodec, hardware: bool) -> String {
    // The session part, then one group per media section
    let mut sections: Vec<Vec<&str>> = vec![Vec::new()];
    for line in sdp.lines() {
        if line.starts_with("m=") {
            sections.push(Vec::new());
        }
        if let Some(section) = sections.last_mut() {
            section.push(line);
        }
    }
    let newline = if sdp.contains("\r\n") { "\r\n" } else { "\n" };
    let mut filtered = sections
        .iter()
        .flat_map(|section| match section.split_first() {
            Some((m, rest)) if m.starts_with("m=video ") => filter_section(m, rest, pick, hardware),
            _ => section.iter().map(|l| (*l).to_owned()).collect(),
        })
        .collect::<Vec<_>>()
        .join(newline);
    if sdp.ends_with('\n') {
        filtered.push_str(newline);
    }
    filtered
}

fn filter_section(m: &str, rest: &[&str], pick: VideoCodec, hardware: bool) -> Vec<String> {
    // m=video <port> <proto> <payload types…>
    let mut fields = m.split(' ');
    let head: Vec<&str> = fields.by_ref().take(3).collect();
    let types: Vec<&str> = fields.collect();
    let attr =
        |prefix: &str, pt: &str| rest.iter().find_map(|l| l.strip_prefix(prefix)?.strip_prefix(pt)?.strip_prefix(' '));
    let name = |pt: &str| {
        attr("a=rtpmap:", pt).and_then(|m| m.split('/').next()).map(str::to_ascii_lowercase).unwrap_or_default()
    };
    let repair = |name: &str| matches!(name, "rtx" | "red" | "ulpfec" | "flexfec-03");
    let kept_codec = |pt: &str| {
        let name = name(pt);
        name != "rtx" && allowed(&name, pick, hardware)
    };
    let keep: Vec<&str> = types
        .iter()
        .copied()
        .filter(|pt| match name(pt).as_str() {
            // Retransmission for a codec that stays
            "rtx" => attr("a=fmtp:", pt)
                .and_then(|f| f.split(';').find_map(|p| p.trim().strip_prefix("apt=")))
                .is_some_and(kept_codec),
            _ => kept_codec(pt),
        })
        .collect();
    let whole = || std::iter::once(m).chain(rest.iter().copied()).map(str::to_owned).collect();
    if keep.len() == types.len() || keep.iter().all(|pt| repair(&name(pt))) {
        return whole();
    }
    let dropped = |line: &str| {
        ["a=rtpmap:", "a=fmtp:", "a=rtcp-fb:"].iter().any(|prefix| {
            line.strip_prefix(prefix)
                .and_then(|rest| rest.split(' ').next())
                .is_some_and(|pt| types.contains(&pt) && !keep.contains(&pt))
        })
    };
    std::iter::once([head, keep.clone()].concat().join(" "))
        .chain(rest.iter().filter(|l| !dropped(l)).map(|l| (*l).to_owned()))
        .collect()
}

/// The codec's name in lowercase: WebRTC writes "video/H264" but "video/rtx"
fn name(codec: &RtpCodecCapability) -> String {
    codec.mime_type.rsplit('/').next().unwrap_or_default().to_ascii_lowercase()
}

/// Whether a parameter of an fmtp line starts with `param`, ignoring case.
fn fmtp_has(line: &str, param: &str) -> bool {
    let starts = |p: &str| p.trim().get(..param.len()).is_some_and(|p| p.eq_ignore_ascii_case(param));
    line.split(';').any(starts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec(mime: &str, fmtp: &str) -> RtpCodecCapability {
        RtpCodecCapability {
            channels: None,
            clock_rate: Some(90_000),
            mime_type: mime.into(),
            sdp_fmtp_line: (!fmtp.is_empty()).then(|| fmtp.into()),
        }
    }

    /// Roughly what LiveKit's factory lists on a Mac, with a mode 0 entry as browsers have
    fn caps() -> Vec<RtpCodecCapability> {
        vec![
            codec("video/VP8", ""),
            codec("video/rtx", ""),
            codec("video/VP9", "profile-id=0"),
            codec("video/H264", "level-asymmetry-allowed=1;packetization-mode=0;profile-level-id=42e01f"),
            codec("video/H264", "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=640c34"),
            codec("video/H264", "level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e034"),
            codec("video/AV1", ""),
            codec("video/red", ""),
            codec("video/ulpfec", ""),
        ]
    }

    fn summary(codecs: &[RtpCodecCapability]) -> Vec<String> {
        codecs
            .iter()
            .map(|c| {
                let profile = c.sdp_fmtp_line.as_deref().and_then(|l| l.split("profile-level-id=").nth(1));
                let mode = c.sdp_fmtp_line.as_deref().and_then(|l| l.split("packetization-mode=").nth(1));
                match (profile, mode) {
                    (Some(p), Some(m)) => format!("{} {} {}", c.mime_type, &p[..6], &m[..1]),
                    _ => c.mime_type.clone(),
                }
            })
            .collect()
    }

    #[test]
    fn auto_with_hardware_puts_constrained_baseline_h264_first() {
        let ordered = summary(&order(caps(), VideoCodec::Auto, true));
        assert_eq!(
            ordered,
            [
                "video/H264 42e034 1",
                "video/H264 640c34 1",
                "video/H264 42e01f 0",
                "video/VP8",
                "video/rtx",
                "video/VP9",
                "video/AV1",
                "video/red",
                "video/ulpfec",
            ]
        );
    }

    #[test]
    fn auto_without_hardware_leaves_h264_out() {
        let ordered = summary(&order(caps(), VideoCodec::Auto, false));
        assert_eq!(ordered, ["video/VP8", "video/rtx", "video/VP9", "video/AV1", "video/red", "video/ulpfec"]);
    }

    #[test]
    fn h264_is_first_even_without_hardware() {
        let ordered = order(caps(), VideoCodec::H264, false);
        assert_eq!(ordered[0].mime_type, "video/H264");
        assert!(ordered.iter().any(|c| c.mime_type == "video/VP8"));
    }

    #[test]
    fn vp8_keeps_only_vp8_and_its_repair_codecs() {
        let ordered = summary(&order(caps(), VideoCodec::Vp8, true));
        assert_eq!(ordered, ["video/VP8", "video/rtx", "video/red", "video/ulpfec"]);
    }

    const OFFER: &str = "v=0\r\n\
        o=- 1 2 IN IP4 127.0.0.1\r\n\
        m=audio 9 UDP/TLS/RTP/SAVPF 111\r\n\
        a=rtpmap:111 opus/48000/2\r\n\
        m=video 9 UDP/TLS/RTP/SAVPF 98 99 100 101 109\r\n\
        a=mid:1\r\n\
        a=rtpmap:98 H264/90000\r\n\
        a=rtcp-fb:98 nack\r\n\
        a=fmtp:98 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
        a=rtpmap:99 rtx/90000\r\n\
        a=fmtp:99 apt=98\r\n\
        a=rtpmap:100 VP8/90000\r\n\
        a=rtcp-fb:100 nack\r\n\
        a=rtpmap:101 rtx/90000\r\n\
        a=fmtp:101 apt=100\r\n\
        a=rtpmap:109 red/90000\r\n\
        a=rtcp-fb:* transport-cc\r\n";

    #[test]
    fn an_offer_loses_h264_and_its_retransmission_when_we_leave_it_out() {
        let filtered = filter_offer(OFFER, VideoCodec::Vp8, true);
        assert!(filtered.contains("m=video 9 UDP/TLS/RTP/SAVPF 100 101 109\r\n"));
        for gone in ["a=rtpmap:98 ", "a=fmtp:98 ", "a=rtcp-fb:98 ", "a=rtpmap:99 ", "a=fmtp:99 "] {
            assert!(!filtered.contains(gone), "{gone} should be gone");
        }
        for kept in ["a=rtpmap:100 VP8", "a=fmtp:101 apt=100", "a=rtcp-fb:* transport-cc", "a=rtpmap:111 opus"] {
            assert!(filtered.contains(kept), "{kept} should stay");
        }
        assert!(filtered.ends_with("\r\n"));
    }

    #[test]
    fn an_offer_we_can_take_whole_is_left_alone() {
        assert_eq!(filter_offer(OFFER, VideoCodec::Auto, true), OFFER);
    }

    #[test]
    fn a_video_section_with_nothing_we_want_is_kept_whole() {
        let h264_only = OFFER.replace("98 99 100 101 109", "98 99");
        assert_eq!(filter_offer(&h264_only, VideoCodec::Vp8, true), h264_only);
    }
}
