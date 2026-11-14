//! How the call copes with loss, per second: NACKs, FEC, retransmissions, freezes. Only the stats log keeps these;
//! the overlay has no room for them.

use libwebrtc::stats::RtcStats;
use serde::Serialize;

/// Running totals from one report, summed over every stream.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Totals {
    nacks_sent: u64,
    nacks_received: u64,
    plis_sent: u64,
    plis_received: u64,
    fec_received: u64,
    fec_discarded: u64,
    rtx_received: u64,
    rtx_sent: u64,
    freezes: u64,
    frames_dropped: u64,
    frames_decoded: u64,
    jitter_buffer_s: f64,
    jitter_buffer_emitted: u64,
    concealed_samples: u64,
    samples_received: u64,
}

/// The change since the previous sample, a second or so before.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Detail {
    /// NACKs we sent: packets of theirs we asked again for
    pub nacks_sent: u64,
    /// NACKs they sent: packets of ours they asked again for
    pub nacks_received: u64,
    /// Keyframe requests we sent, after a loss NACK couldn't fix
    pub plis_sent: u64,
    pub plis_received: u64,
    /// FEC packets that arrived, and those of them that weren't needed or came too late
    pub fec_received: u64,
    pub fec_discarded: u64,
    /// Retransmitted packets that reached us, and that we sent
    pub rtx_received: u64,
    pub rtx_sent: u64,
    /// Their video stopping for longer than it should between frames. How long isn't known: `libwebrtc`'s
    /// `total_freeze_duration` never fills, its name doesn't match WebRTC's `totalFreezesDuration`
    pub freezes: u64,
    pub frames_dropped: u64,
    pub frames_decoded: u64,
    /// How long their video frames waited in the jitter buffer, on average
    pub jitter_buffer_ms: Option<f64>,
    /// Their sound we had to make up, lost or late
    pub concealed_pct: Option<f64>,
    /// What the encoders are aiming for, together
    pub target_kbps: f64,
}

pub(crate) fn totals(report: &[RtcStats]) -> (Totals, f64) {
    let mut t = Totals::default();
    let mut target_bps = 0.0;
    for s in report {
        match s {
            RtcStats::InboundRtp(r) => {
                let i = &r.inbound;
                t.nacks_sent += u64::from(i.nack_count);
                t.plis_sent += u64::from(i.pli_count);
                t.fec_received += i.fec_packets_received;
                t.fec_discarded += i.fec_packets_discarded;
                t.rtx_received += i.retransmitted_packets_received;
                if r.stream.kind == "video" {
                    t.freezes += u64::from(i.freeze_count);
                    t.frames_dropped += u64::from(i.frames_dropped);
                    t.frames_decoded += u64::from(i.frames_decoded);
                    t.jitter_buffer_s += i.jitter_buffer_delay;
                    t.jitter_buffer_emitted += i.jitter_buffer_emitted_count;
                } else {
                    t.concealed_samples += i.concealed_samples;
                    t.samples_received += i.total_samples_received;
                }
            }
            RtcStats::OutboundRtp(r) => {
                let o = &r.outbound;
                t.nacks_received += u64::from(o.nack_count);
                t.plis_received += u64::from(o.pli_count);
                t.rtx_sent += o.retransmitted_packets_sent;
                if r.stream.kind == "video" && o.active {
                    target_bps += o.target_bitrate;
                }
            }
            _ => {}
        }
    }
    (t, target_bps / 1000.0)
}

pub(crate) fn detail(now: &Totals, prev: &Totals, target_kbps: f64) -> Detail {
    let jitter_buffer_emitted = now.jitter_buffer_emitted.saturating_sub(prev.jitter_buffer_emitted);
    let samples = now.samples_received.saturating_sub(prev.samples_received);
    Detail {
        nacks_sent: now.nacks_sent.saturating_sub(prev.nacks_sent),
        nacks_received: now.nacks_received.saturating_sub(prev.nacks_received),
        plis_sent: now.plis_sent.saturating_sub(prev.plis_sent),
        plis_received: now.plis_received.saturating_sub(prev.plis_received),
        fec_received: now.fec_received.saturating_sub(prev.fec_received),
        fec_discarded: now.fec_discarded.saturating_sub(prev.fec_discarded),
        rtx_received: now.rtx_received.saturating_sub(prev.rtx_received),
        rtx_sent: now.rtx_sent.saturating_sub(prev.rtx_sent),
        freezes: now.freezes.saturating_sub(prev.freezes),
        frames_dropped: now.frames_dropped.saturating_sub(prev.frames_dropped),
        frames_decoded: now.frames_decoded.saturating_sub(prev.frames_decoded),
        jitter_buffer_ms: (jitter_buffer_emitted > 0)
            .then(|| (now.jitter_buffer_s - prev.jitter_buffer_s).max(0.0) * 1000.0 / jitter_buffer_emitted as f64),
        concealed_pct: (samples > 0)
            .then(|| now.concealed_samples.saturating_sub(prev.concealed_samples) as f64 * 100.0 / samples as f64),
        target_kbps,
    }
}
