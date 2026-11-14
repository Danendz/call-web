//! The call with the other person: what the server says about them, the peer connection to them, and what we send
//! them besides the tracks (`media` messages, our shared screen).

use std::time::{Duration, Instant};

use libwebrtc::peer_connection::IceConnectionState;
use protocol::{ClientMessage, ServerMessage};

use super::{ChatEvent, Controller, Internal, Link, Phase};
use crate::peer::{Peer, PeerConfig, PeerEvent};
use crate::stats::{CallStats, Path};

// While a call is relayed, retry direct after these delays (the last one repeats). Direct is ~2x faster than going
// through the relay, and a P2P attempt that failed at call start often works a few minutes later.
const DIRECT_RETRY_DELAYS: [Duration; 4] =
    [Duration::from_secs(15), Duration::from_secs(30), Duration::from_secs(60), Duration::from_secs(120)];
const TRYING_DIRECT_SHOWN: Duration = Duration::from_secs(5);

const PEER_SCREENSHOT_SHOWN: Duration = Duration::from_secs(3);

impl Controller {
    pub(super) fn server_message(&mut self, msg: ServerMessage) {
        match msg {
            ServerMessage::Joined {
                client_id,
                token,
                resumed,
                peer_present,
                polite,
                ice_servers,
                chat,
                call_elapsed_ms,
            } => {
                (self.on_chat)(ChatEvent::Reset { my_id: client_id.clone(), messages: chat });
                self.seat.token = Some(token.clone());
                if let Some(signaling) = &self.signaling {
                    signaling.set_identity(client_id, token);
                }
                self.seat.ice_servers = ice_servers;
                if resumed && self.peer.is_some() {
                    // Our socket dropped (network switch); the media path likely died with it
                    if peer_present {
                        if let Some(peer) = &self.peer {
                            peer.restart_ice();
                        }
                        self.send_media(); // in case an update was lost with the socket
                    } else {
                        self.drop_peer();
                    }
                } else {
                    self.seat.polite = polite;
                    if peer_present {
                        self.create_peer();
                    }
                }
                if self.peer.is_some() {
                    self.set_call_elapsed(call_elapsed_ms);
                } else {
                    self.state.phase = Phase::Waiting;
                }
            }
            ServerMessage::PeerJoined { polite, call_elapsed_ms } => {
                self.seat.polite = polite;
                self.create_peer();
                self.set_call_elapsed(Some(call_elapsed_ms));
            }
            ServerMessage::PeerLeft => {
                self.drop_peer();
                self.state.remote_cam_on = true;
                self.state.remote_speaker_off = false;
            }
            ServerMessage::Signal { payload } => {
                if let Some(peer) = &self.peer {
                    peer.handle_signal(payload);
                }
            }
            ServerMessage::Media { cam_on, screen, speaker_off } => {
                self.state.remote_cam_on = cam_on;
                self.state.remote_speaker_off = speaker_off;
                self.remote.set_screen(screen);
            }
            ServerMessage::Chat { message } => (self.on_chat)(ChatEvent::Added(message)),
            ServerMessage::ChatTranslation { id, translation } => {
                (self.on_chat)(ChatEvent::Translated { id, translation });
            }
            ServerMessage::ChatLive { id, text, .. } => (self.on_chat)(ChatEvent::Live { id, text }),
            ServerMessage::Screenshot => {
                self.state.peer_screenshot = true;
                let tx = self.internal_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(PEER_SCREENSHOT_SHOWN).await;
                    let _ = tx.send(Internal::PeerScreenshotShown);
                });
            }
            ServerMessage::Full { reason } => {
                self.signaling = None;
                self.drop_peer();
                self.state.phase = Phase::Full;
                self.state.full_reason = reason;
            }
            ServerMessage::Error { message } => tracing::warn!("server error: {message}"),
            // Chat and the rest arrive with the milestones that use them
            _ => {}
        }
    }

    pub(super) fn create_peer(&mut self) {
        self.drop_peer();
        let config = PeerConfig {
            ice_servers: self.seat.ice_servers.clone(),
            force_relay: self.state.force_relay,
            polite: self.seat.polite,
            stream_id: self.stream_id.clone(),
            screen_stream_id: self.screen.stream_id.clone(),
            stats_shown: self.stats_shown,
        };
        match Peer::start(self.media.factory(), config, self.mic.clone(), self.camera.track(), self.peer_tx.clone()) {
            Ok(peer) => {
                self.peer = Some(peer);
                self.direct_retries = 0;
                self.next_direct_retry = None;
                self.state.phase = Phase::InCall;
                self.state.link = Some(Link::Connecting);
                self.send_media();
                // A new connection while sharing (they rejoined) picks the share up again
                if self.screen.sharing()
                    && let Some(peer) = &self.peer
                {
                    peer.set_screen(Some(self.screen.tracks()));
                }
            }
            Err(e) => {
                tracing::error!("couldn't create the peer connection: {}", e.message);
                self.state.phase = Phase::Waiting;
            }
        }
    }

    pub(super) fn drop_peer(&mut self) {
        self.peer = None;
        self.remote_audio.clear();
        self.remote.drop_tracks();
        (self.on_stats)(None);
        // Events from the old connection still queued must not touch the next one
        while self.peer_rx.try_recv().is_ok() {}
        self.state.link = None;
        self.state.call_started = None;
        if self.state.phase == Phase::InCall {
            self.state.phase = Phase::Waiting;
        }
    }

    pub(super) fn set_call_elapsed(&mut self, ms: Option<u64>) {
        let Some(ms) = ms else { return };
        let now = Instant::now();
        self.state.call_started = Some(now.checked_sub(Duration::from_millis(ms)).unwrap_or(now));
    }

    /// Tells the other side what we're sending, which the tracks alone can't say: a camera turned off still sends
    /// black frames.
    pub(super) fn send_media(&self) {
        if self.peer.is_none() {
            return;
        }
        if let Some(signaling) = &self.signaling {
            let screen = self.screen.sharing().then(|| self.screen.stream_id.clone());
            signaling.send(ClientMessage::Media {
                cam_on: self.camera.sending(),
                screen,
                speaker_off: self.state.speaker_off,
            });
        }
    }

    pub(super) fn start_share(&mut self, source: Option<String>) {
        self.state.screen_picker = None;
        let tx = self.internal_tx.clone();
        self.screen.start(source, move || {
            let _ = tx.send(Internal::ScreenEnded);
        });
        // Announced first, so the other side knows the screen's stream before its track arrives
        self.send_media();
        if let Some(peer) = &self.peer {
            peer.set_screen(Some(self.screen.tracks()));
        }
    }

    pub(super) fn stop_share(&mut self) {
        if !self.screen.sharing() {
            return;
        }
        self.screen.stop();
        self.send_media();
        if let Some(peer) = &self.peer {
            peer.set_screen(None);
        }
    }

    pub(super) fn peer_event(&mut self, event: PeerEvent) {
        match event {
            PeerEvent::Signal(payload) => {
                if let Some(signaling) = &self.signaling {
                    signaling.send(ClientMessage::Signal { payload });
                }
            }
            PeerEvent::RemoteVideo { stream_id, track } => self.remote.add(stream_id, track),
            PeerEvent::RemoteAudio(track) => {
                track.set_enabled(!self.state.speaker_off);
                self.remote_audio.push(track);
            }
            PeerEvent::Stats(stats) => {
                self.maybe_retry_direct(&stats);
                if self.stats_shown {
                    let trying_direct = self.last_direct_retry.is_some_and(|at| at.elapsed() < TRYING_DIRECT_SHOWN);
                    (self.on_stats)(Some((*stats, trying_direct)));
                }
            }
            PeerEvent::IceState(ice) => {
                self.state.link = Some(match ice {
                    IceConnectionState::Connected | IceConnectionState::Completed => Link::Connected,
                    IceConnectionState::Disconnected => Link::Unstable,
                    IceConnectionState::Failed => Link::Failed,
                    _ => Link::Connecting,
                });
            }
        }
    }

    /// Upgrades a relayed call to direct P2P with an ICE restart, backing off between tries. Only the impolite side
    /// drives it, so the two sides don't restart over each other.
    pub(super) fn maybe_retry_direct(&mut self, stats: &CallStats) {
        if stats.path == Path::Direct || self.state.force_relay || self.seat.polite {
            self.direct_retries = 0;
            self.next_direct_retry = None;
            return;
        }
        if stats.path != Path::Relay {
            return;
        }
        let now = Instant::now();
        let Some(next) = self.next_direct_retry else {
            self.next_direct_retry = Some(now + DIRECT_RETRY_DELAYS[0]);
            return;
        };
        if now < next {
            return;
        }
        self.direct_retries += 1;
        let delay =
            DIRECT_RETRY_DELAYS.get(self.direct_retries).or(DIRECT_RETRY_DELAYS.last()).copied().unwrap_or_default();
        self.next_direct_retry = Some(now + delay);
        self.last_direct_retry = Some(now);
        if let Some(peer) = &self.peer {
            peer.restart_ice();
        }
    }
}
