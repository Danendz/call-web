//! WebSocket to the room with heartbeat and automatic reconnect, like the web's `signaling.ts`. Reconnects send
//! `clientId` and `token` so the server resumes our seat.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{ClientMessage, ServerMessage};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use url::Url;

const PING_EVERY: Duration = Duration::from_secs(10);
// No pong for this long means the socket is dead even if the OS hasn't noticed (common after a network switch)
const DEAD_AFTER: Duration = Duration::from_secs(25);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(8);
// The server keeps 200 chat messages of up to 2000 characters, each with two translations: `joined` stays under this
const MAX_MESSAGE_BYTES: usize = 8 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketStatus {
    Connecting,
    Open,
    Reconnecting,
}

#[derive(Debug)]
pub enum SignalEvent {
    Status(SocketStatus),
    Message(ServerMessage),
}

enum Outgoing {
    Send(ClientMessage),
    SetIdentity { client_id: String, token: String },
}

/// Runs until dropped. Dropping it closes the socket; [`Signaling::leave`] first tells the server we left on purpose.
pub struct Signaling {
    out: mpsc::UnboundedSender<Outgoing>,
}

impl Signaling {
    pub fn connect(server: &Url, room_id: &str, events: mpsc::UnboundedSender<SignalEvent>) -> Self {
        let (out, out_rx) = mpsc::unbounded_channel();
        let socket = Socket { base: ws_url(server, room_id), identity: None, events, out: out_rx };
        tokio::spawn(socket.run());
        Self { out }
    }

    pub fn set_identity(&self, client_id: String, token: String) {
        let _ = self.out.send(Outgoing::SetIdentity { client_id, token });
    }

    /// Dropped while the socket is down, as in the web client: what matters is resent on reconnect.
    pub fn send(&self, msg: ClientMessage) {
        let _ = self.out.send(Outgoing::Send(msg));
    }

    pub fn leave(self) {
        self.send(ClientMessage::Leave);
    }
}

fn ws_url(server: &Url, room_id: &str) -> Url {
    let mut url = server.clone();
    // Only http and https servers are accepted at startup, so this can't fail
    let _ = url.set_scheme(if server.scheme() == "https" { "wss" } else { "ws" });
    url.set_path("/ws");
    url.query_pairs_mut().clear().append_pair("room", room_id);
    url
}

struct Socket {
    base: Url,
    identity: Option<(String, String)>,
    events: mpsc::UnboundedSender<SignalEvent>,
    out: mpsc::UnboundedReceiver<Outgoing>,
}

enum Ended {
    /// The app dropped us, left, or the room is full: don't reconnect
    Stop,
    Lost,
}

impl Socket {
    async fn run(mut self) {
        let mut attempt = 0u32;
        loop {
            let status = if attempt == 0 { SocketStatus::Connecting } else { SocketStatus::Reconnecting };
            self.emit(SignalEvent::Status(status));
            if let Ended::Stop = self.session(&mut attempt).await {
                return;
            }
            self.emit(SignalEvent::Status(SocketStatus::Reconnecting));
            let backoff = Duration::from_secs(1u64 << attempt.min(3)).min(MAX_BACKOFF);
            attempt = attempt.saturating_add(1);
            if let Ended::Stop = self.wait(backoff).await {
                return;
            }
        }
    }

    /// One socket from connect to close.
    async fn session(&mut self, attempt: &mut u32) -> Ended {
        let mut url = self.base.clone();
        if let Some((client_id, token)) = &self.identity {
            url.query_pairs_mut().append_pair("clientId", client_id).append_pair("token", token);
        }
        let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE_BYTES));
        let connect = tokio_tungstenite::connect_async_with_config(url.as_str(), Some(config), false);
        let mut ws = match timeout(CONNECT_TIMEOUT, connect).await {
            Ok(Ok((ws, _))) => ws,
            Ok(Err(e)) => {
                tracing::info!("signaling connect failed: {e}");
                return Ended::Lost;
            }
            Err(_) => {
                tracing::info!("signaling connect timed out");
                return Ended::Lost;
            }
        };

        *attempt = 0;
        self.emit(SignalEvent::Status(SocketStatus::Open));
        let mut last_heard = Instant::now();
        let mut ping = interval(PING_EVERY);
        ping.reset();

        loop {
            tokio::select! {
                incoming = ws.next() => {
                    let text = match incoming {
                        Some(Ok(Message::Text(text))) => text,
                        Some(Ok(Message::Close(_))) | None => return Ended::Lost,
                        Some(Ok(_)) => continue,
                        Some(Err(e)) => {
                            tracing::info!("signaling socket error: {e}");
                            return Ended::Lost;
                        }
                    };
                    last_heard = Instant::now();
                    let msg = match serde_json::from_str::<ServerMessage>(&text) {
                        Ok(ServerMessage::Pong) => continue,
                        Ok(ServerMessage::Unknown) => {
                            tracing::debug!("ignoring a server message this build doesn't know");
                            continue;
                        }
                        Ok(msg) => msg,
                        Err(e) => {
                            tracing::warn!("unreadable server message: {e}");
                            continue;
                        }
                    };
                    let full = matches!(msg, ServerMessage::Full { .. });
                    self.emit(SignalEvent::Message(msg));
                    if full {
                        return Ended::Stop;
                    }
                }
                out = self.out.recv() => match out {
                    Some(Outgoing::Send(msg)) => {
                        let leaving = matches!(msg, ClientMessage::Leave);
                        if let Ok(json) = serde_json::to_string(&msg)
                            && ws.send(Message::text(json)).await.is_err()
                        {
                            return Ended::Lost;
                        }
                        if leaving {
                            let _ = ws.close(None).await;
                            return Ended::Stop;
                        }
                    }
                    Some(Outgoing::SetIdentity { client_id, token }) => self.identity = Some((client_id, token)),
                    None => {
                        let _ = ws.close(None).await;
                        return Ended::Stop;
                    }
                },
                _ = ping.tick() => {
                    if last_heard.elapsed() > DEAD_AFTER {
                        tracing::info!("signaling socket went quiet, reconnecting");
                        return Ended::Lost;
                    }
                    if let Ok(json) = serde_json::to_string(&ClientMessage::Ping)
                        && ws.send(Message::text(json)).await.is_err()
                    {
                        return Ended::Lost;
                    }
                }
            }
        }
    }

    /// Sleeps before the next attempt, still taking identity updates; messages sent meanwhile are dropped.
    async fn wait(&mut self, delay: Duration) -> Ended {
        let until = sleep(delay);
        tokio::pin!(until);
        loop {
            tokio::select! {
                () = &mut until => return Ended::Lost,
                out = self.out.recv() => match out {
                    Some(Outgoing::SetIdentity { client_id, token }) => self.identity = Some((client_id, token)),
                    Some(Outgoing::Send(ClientMessage::Leave)) | None => return Ended::Stop,
                    Some(Outgoing::Send(_)) => {}
                },
            }
        }
    }

    fn emit(&self, event: SignalEvent) {
        let _ = self.events.send(event);
    }
}
