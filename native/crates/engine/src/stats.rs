//! What the stats overlay shows, read from WebRTC's report once a second, like the web's `stats.ts`.

use std::time::Instant;

use libwebrtc::stats::{IceCandidateType, IceServerTransportProtocol, QualityLimitationReason, RtcStats};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Direct,
    Relay,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct VideoStats {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    /// "VP8", "H264"…
    pub codec: Option<String>,
    /// The encoder or decoder doing the work, e.g. "libvpx" or a hardware one
    pub implementation: Option<String>,
    /// What holds the outgoing picture back: "cpu" or "bandwidth"
    pub limited_by: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CallStats {
    pub path: Path,
    pub local_type: Option<&'static str>,
    pub remote_type: Option<&'static str>,
    pub protocol: Option<String>,
    /// When we're the one relaying: how we reach the TURN server
    pub relay_protocol: Option<&'static str>,
    pub rtt_ms: Option<f64>,
    pub in_kbps: f64,
    pub out_kbps: f64,
    /// What we receive
    pub loss_pct: Option<f64>,
    /// What we send, as the other side reports it back
    pub loss_out_pct: Option<f64>,
    pub jitter_ms: Option<f64>,
    pub video_in: Option<VideoStats>,
    pub video_out: Option<VideoStats>,
    /// WebRTC's estimate of how much it can send
    pub send_budget_kbps: Option<f64>,
    /// Converting a frame for the screen, smoothed: the CPU render path's cost
    pub convert_ms: f64,
}

/// Running totals, kept between samples to turn them into rates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Counters {
    at: Instant,
    bytes_in: u64,
    bytes_out: u64,
    packets_in: u64,
    packets_lost: i64,
}

pub(crate) fn sample(report: &[RtcStats], prev: Option<&Counters>, now: Instant) -> (CallStats, Counters) {
    let selected = report.iter().find_map(|s| match s {
        RtcStats::Transport(t) if !t.transport.selected_candidate_pair_id.is_empty() => {
            Some(t.transport.selected_candidate_pair_id.as_str())
        }
        _ => None,
    });
    let pair = report.iter().find_map(|s| match s {
        RtcStats::CandidatePair(p) if Some(p.rtc.id.as_str()) == selected => Some(&p.candidate_pair),
        _ => None,
    });
    let local = pair.and_then(|p| {
        report.iter().find_map(|s| match s {
            RtcStats::LocalCandidate(c) if c.rtc.id == p.local_candidate_id => Some(&c.local_candidate),
            _ => None,
        })
    });
    let remote = pair.and_then(|p| {
        report.iter().find_map(|s| match s {
            RtcStats::RemoteCandidate(c) if c.rtc.id == p.remote_candidate_id => Some(&c.remote_candidate),
            _ => None,
        })
    });

    let codec = |id: &str| {
        report.iter().find_map(|s| match s {
            RtcStats::Codec(c) if c.rtc.id == id => {
                let mime = c.codec.mime_type.as_str();
                Some(mime.split_once('/').map_or(mime, |(_, name)| name).to_owned())
            }
            _ => None,
        })
    };
    let non_empty = |s: &str| Some(s.to_owned()).filter(|s| !s.is_empty());

    let mut counters = Counters { at: now, bytes_in: 0, bytes_out: 0, packets_in: 0, packets_lost: 0 };
    let (mut jitter_ms, mut video_in, mut video_out, mut loss_out_pct) = (None, None, None, None::<f64>);
    for s in report {
        match s {
            RtcStats::InboundRtp(r) => {
                counters.bytes_in += r.inbound.bytes_received;
                counters.packets_in += r.received.packets_received;
                counters.packets_lost += r.received.packets_lost;
                if r.stream.kind == "audio" {
                    jitter_ms = Some(r.received.jitter * 1000.0);
                } else if r.stream.kind == "video" && r.inbound.frame_width > 0 {
                    let video = VideoStats {
                        width: r.inbound.frame_width,
                        height: r.inbound.frame_height,
                        fps: r.inbound.frames_per_second,
                        codec: codec(&r.stream.codec_id),
                        implementation: non_empty(&r.inbound.decoder_implementation),
                        limited_by: None,
                    };
                    video_in = Some(bigger(video_in, video));
                }
            }
            RtcStats::OutboundRtp(r) => {
                counters.bytes_out += r.sent.bytes_sent;
                if r.stream.kind == "video" && r.outbound.frame_width > 0 {
                    let video = VideoStats {
                        width: r.outbound.frame_width,
                        height: r.outbound.frame_height,
                        fps: r.outbound.frames_per_second,
                        codec: codec(&r.stream.codec_id),
                        implementation: non_empty(&r.outbound.encoder_implementation),
                        limited_by: match r.outbound.quality_limitation_reason {
                            QualityLimitationReason::Cpu => Some("cpu"),
                            QualityLimitationReason::Bandwidth => Some("bandwidth"),
                            QualityLimitationReason::Other => Some("other"),
                            QualityLimitationReason::None => None,
                        },
                    };
                    video_out = Some(bigger(video_out, video));
                }
            }
            RtcStats::RemoteInboundRtp(r) => {
                let lost = r.remote_inbound.fraction_lost * 100.0;
                loss_out_pct = Some(loss_out_pct.map_or(lost, |l| l.max(lost)));
            }
            _ => {}
        }
    }

    let (mut in_kbps, mut out_kbps, mut loss_pct) = (0.0, 0.0, None);
    if let Some(prev) = prev {
        let dt = now.duration_since(prev.at).as_secs_f64();
        if dt > 0.0 {
            in_kbps = counters.bytes_in.saturating_sub(prev.bytes_in) as f64 * 8.0 / 1000.0 / dt;
            out_kbps = counters.bytes_out.saturating_sub(prev.bytes_out) as f64 * 8.0 / 1000.0 / dt;
        }
        let lost = counters.packets_lost - prev.packets_lost;
        let total = counters.packets_in.saturating_sub(prev.packets_in) as i64 + lost;
        if total > 0 {
            loss_pct = Some((lost as f64 / total as f64 * 100.0).max(0.0));
        }
    }

    let kind = |c: Option<IceCandidateType>| match c {
        Some(IceCandidateType::Host) => Some("host"),
        Some(IceCandidateType::Srflx) => Some("srflx"),
        Some(IceCandidateType::Prflx) => Some("prflx"),
        Some(IceCandidateType::Relay) => Some("relay"),
        None => None,
    };
    let local_type = kind(local.and_then(|c| c.candidate_type));
    let remote_type = kind(remote.and_then(|c| c.candidate_type));
    let path = match pair {
        None => Path::Unknown,
        Some(_) if local_type == Some("relay") || remote_type == Some("relay") => Path::Relay,
        Some(_) => Path::Direct,
    };
    let stats = CallStats {
        path,
        local_type,
        remote_type,
        protocol: local.map(|c| c.protocol.clone()).filter(|p| !p.is_empty()),
        relay_protocol: local.and_then(|c| c.relay_protocol).map(|p| match p {
            IceServerTransportProtocol::Udp => "udp",
            IceServerTransportProtocol::Tcp => "tcp",
            IceServerTransportProtocol::Tls => "tls",
        }),
        rtt_ms: pair.map(|p| p.current_round_trip_time * 1000.0).filter(|rtt| *rtt > 0.0),
        in_kbps,
        out_kbps,
        loss_pct,
        loss_out_pct,
        jitter_ms,
        video_in,
        video_out,
        send_budget_kbps: pair.map(|p| p.available_outgoing_bitrate / 1000.0).filter(|b| *b > 0.0),
        convert_ms: f64::from(crate::video::CONVERT_US.get()) / 1000.0,
    };
    (stats, counters)
}

/// While sharing, the camera and the screen both send: the overlay shows the bigger picture, which is the screen.
fn bigger(current: Option<VideoStats>, next: VideoStats) -> VideoStats {
    match current {
        Some(c) if u64::from(c.width) * u64::from(c.height) >= u64::from(next.width) * u64::from(next.height) => c,
        _ => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(json: &str) -> Vec<RtcStats> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn relay_path_and_rates_from_two_samples() {
        let first = report(
            r#"[
            {"type":"transport","id":"T","timestamp":0,"selectedCandidatePairId":"P"},
            {"type":"candidate-pair","id":"P","timestamp":0,"localCandidateId":"L","remoteCandidateId":"R",
             "currentRoundTripTime":0.21,"availableOutgoingBitrate":1500000},
            {"type":"local-candidate","id":"L","timestamp":0,"candidateType":"relay","protocol":"udp","relayProtocol":"tls"},
            {"type":"remote-candidate","id":"R","timestamp":0,"candidateType":"srflx","protocol":"udp"},
            {"type":"inbound-rtp","id":"I","timestamp":0,"kind":"audio","bytesReceived":1000,"packetsReceived":90,
             "packetsLost":0,"jitter":0.012}
        ]"#,
        );
        let t0 = Instant::now();
        let (stats, counters) = sample(&first, None, t0);
        assert_eq!(stats.path, Path::Relay);
        assert_eq!(stats.relay_protocol, Some("tls"));
        assert_eq!(stats.rtt_ms.map(f64::round), Some(210.0));
        assert_eq!(stats.loss_pct, None, "no rate without a previous sample");

        let second = report(
            r#"[
            {"type":"inbound-rtp","id":"I","timestamp":0,"kind":"audio","bytesReceived":126000,"packetsReceived":180,
             "packetsLost":10,"jitter":0.012}
        ]"#,
        );
        let (stats, _) = sample(&second, Some(&counters), t0 + std::time::Duration::from_secs(1));
        assert_eq!(stats.path, Path::Unknown, "no selected pair in this report");
        assert_eq!(stats.in_kbps.round(), 1000.0);
        assert_eq!(stats.loss_pct.map(|l| l.round()), Some(10.0));
    }

    #[test]
    fn video_out_is_the_shared_screen_with_its_codec() {
        let both = report(
            r#"[
            {"type":"codec","id":"C","timestamp":0,"mimeType":"video/VP8"},
            {"type":"outbound-rtp","id":"S","timestamp":0,"kind":"video","codecId":"C","frameWidth":1920,
             "frameHeight":1080,"framesPerSecond":60,"encoderImplementation":"libvpx"},
            {"type":"outbound-rtp","id":"K","timestamp":0,"kind":"video","codecId":"C","frameWidth":640,
             "frameHeight":360,"framesPerSecond":30,"encoderImplementation":"libvpx"}
        ]"#,
        );
        let (stats, _) = sample(&both, None, Instant::now());
        let out = stats.video_out.unwrap();
        assert_eq!((out.width, out.height), (1920, 1080), "the screen, not the camera after it");
        assert_eq!(out.codec.as_deref(), Some("VP8"));
        assert_eq!(out.implementation.as_deref(), Some("libvpx"));
    }
}
