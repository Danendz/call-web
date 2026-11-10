//! `/ws/user` with heartbeat and automatic reconnect, like the web's `friendsSocket.ts`.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{UserClientMessage, UserServerMessage};
use tokio::sync::mpsc;
use tokio::time::{Instant, interval, sleep, timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use url::Url;

use super::SessionToken;

const PING_EVERY: Duration = Duration::from_secs(20);
const DEAD_AFTER: Duration = Duration::from_secs(45);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_BACKOFF: Duration = Duration::from_secs(15);
/// The server closes with this when the session was signed out, here or on another device
const SIGNED_OUT: u16 = 4401;

#[derive(Debug)]
pub enum SocketEvent {
    Open,
    Lost,
    /// The session is gone: stop, and sign out here
    SignedOut,
    Message(UserServerMessage),
}

/// Runs until dropped.
pub struct UserSocket {
    out: mpsc::UnboundedSender<UserClientMessage>,
}

impl UserSocket {
    pub fn connect(server: &Url, session: SessionToken, events: mpsc::UnboundedSender<SocketEvent>) -> Self {
        let (out, out_rx) = mpsc::unbounded_channel();
        let url = crate::ws::ws_url(server, "/ws/user");
        tokio::spawn(run(url, session, events, out_rx));
        Self { out }
    }

    /// Dropped while the socket is down: the next state after reconnecting shows where things stand.
    pub fn send(&self, msg: UserClientMessage) {
        let _ = self.out.send(msg);
    }
}

enum Ended {
    Stop,
    Lost { opened: bool },
}

async fn run(
    url: Url,
    session: SessionToken,
    events: mpsc::UnboundedSender<SocketEvent>,
    mut out: mpsc::UnboundedReceiver<UserClientMessage>,
) {
    let mut attempt = 0u32;
    loop {
        match session_loop(&url, &session, &events, &mut out).await {
            Ended::Stop => return,
            Ended::Lost { opened } => {
                if opened {
                    attempt = 0;
                }
                let _ = events.send(SocketEvent::Lost);
            }
        }
        let backoff = Duration::from_secs(1u64 << attempt.min(4)).min(MAX_BACKOFF);
        attempt = attempt.saturating_add(1);
        let until = sleep(backoff);
        tokio::pin!(until);
        loop {
            tokio::select! {
                () = &mut until => break,
                msg = out.recv() => if msg.is_none() { return },
            }
        }
    }
}

async fn session_loop(
    url: &Url,
    session: &SessionToken,
    events: &mpsc::UnboundedSender<SocketEvent>,
    out: &mut mpsc::UnboundedReceiver<UserClientMessage>,
) -> Ended {
    let request = match crate::ws::request(url, Some(&session.0)) {
        Ok(request) => request,
        Err(e) => {
            tracing::warn!("user socket request: {e}");
            return Ended::Stop;
        }
    };
    let mut ws = match timeout(CONNECT_TIMEOUT, tokio_tungstenite::connect_async(request)).await {
        Ok(Ok((ws, _))) => ws,
        Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if response.status() == 401 => {
            let _ = events.send(SocketEvent::SignedOut);
            return Ended::Stop;
        }
        Ok(Err(e)) => {
            tracing::info!("user socket connect failed: {e}");
            return Ended::Lost { opened: false };
        }
        Err(_) => return Ended::Lost { opened: false },
    };
    let _ = events.send(SocketEvent::Open);
    let mut last_heard = Instant::now();
    let mut ping = interval(PING_EVERY);
    ping.reset();
    loop {
        tokio::select! {
            incoming = ws.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(text))) => text,
                    Some(Ok(Message::Close(frame))) => {
                        if frame.is_some_and(|f| f.code == CloseCode::from(SIGNED_OUT)) {
                            let _ = events.send(SocketEvent::SignedOut);
                            return Ended::Stop;
                        }
                        return Ended::Lost { opened: true };
                    }
                    None => return Ended::Lost { opened: true },
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        tracing::info!("user socket error: {e}");
                        return Ended::Lost { opened: true };
                    }
                };
                last_heard = Instant::now();
                match serde_json::from_str::<UserServerMessage>(&text) {
                    Ok(UserServerMessage::Pong | UserServerMessage::Unknown) => {}
                    Ok(msg) => { let _ = events.send(SocketEvent::Message(msg)); }
                    Err(e) => tracing::warn!("unreadable user socket message: {e}"),
                }
            }
            msg = out.recv() => match msg {
                Some(msg) => {
                    if let Ok(json) = serde_json::to_string(&msg)
                        && ws.send(Message::text(json)).await.is_err()
                    {
                        return Ended::Lost { opened: true };
                    }
                }
                None => {
                    let _ = ws.close(None).await;
                    return Ended::Stop;
                }
            },
            _ = ping.tick() => {
                if last_heard.elapsed() > DEAD_AFTER {
                    return Ended::Lost { opened: true };
                }
                if let Ok(json) = serde_json::to_string(&UserClientMessage::Ping)
                    && ws.send(Message::text(json)).await.is_err()
                {
                    return Ended::Lost { opened: true };
                }
            }
        }
    }
}
