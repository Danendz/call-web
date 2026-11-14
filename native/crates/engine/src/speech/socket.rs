//! `/ws/speech`, kept apart from signaling like the web's `speechSocket.ts`, so a stalled audio frame on a lossy link
//! never holds up the call's own messages. Utterances go out while they're said; what's sent while the socket is
//! still connecting waits for it, up to a cap.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use protocol::{MAX_UTTERANCE_S, SpeechClientMessage, SpeechServerMessage, TRANSCRIBE_SAMPLE_RATE};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use url::Url;

use super::{Out, Recognizer, SpeechEvent};

// After a drop, the next utterance reconnects; not sooner than this, so a server that's down isn't hammered
const RETRY: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
// What may wait for the socket to open: about one long utterance
const MAX_QUEUED_BYTES: usize = (MAX_UTTERANCE_S * TRANSCRIBE_SAMPLE_RATE * 2) as usize;

/// Runs until dropped. Sending never blocks: the socket task does the waiting.
pub(crate) struct SpeechSocket {
    out: mpsc::UnboundedSender<Out>,
}

impl SpeechSocket {
    /// `url` is the full `/ws/speech?room=&token=` address; the server's notices come back through `notices`.
    pub(crate) fn connect(url: Url, notices: mpsc::UnboundedSender<SpeechEvent>) -> Self {
        let (out, rx) = mpsc::unbounded_channel();
        tokio::spawn(run(url, rx, notices));
        Self { out }
    }
}

impl Recognizer for SpeechSocket {
    fn send(&self, msg: Out) {
        let _ = self.out.send(msg);
    }
}

async fn run(url: Url, mut out: mpsc::UnboundedReceiver<Out>, notices: mpsc::UnboundedSender<SpeechEvent>) {
    let mut retry_at = Instant::now();
    // The first connect goes out at once, like the web's connect(): the first utterance doesn't wait for the
    // handshake, and a server without speech says so before anyone speaks. After a drop, the next utterance
    // reconnects.
    let mut eager = true;
    'connect: loop {
        let mut queue = Vec::new();
        if !std::mem::take(&mut eager) {
            let Some(first) = out.recv().await else { return };
            queue.push(first);
        }
        let mut queued = 0usize;
        tokio::time::sleep_until(retry_at).await;
        let connect = tokio_tungstenite::connect_async(url.as_str());
        let ws = tokio::select! {
            result = tokio::time::timeout(CONNECT_TIMEOUT, connect) => result,
            // Keep accepting while connecting, up to the cap; an utterance that doesn't fit is dropped whole
            () = async {
                while let Some(msg) = out.recv().await {
                    queued += size(&msg);
                    if queued <= MAX_QUEUED_BYTES {
                        queue.push(msg);
                    }
                }
            } => return,
        };
        let mut ws = match ws {
            Ok(Ok((ws, _))) => ws,
            _ => {
                tracing::info!("speech socket didn't connect");
                retry_at = Instant::now() + RETRY;
                continue 'connect;
            }
        };
        for msg in queue {
            if ws.send(encode(msg)).await.is_err() {
                retry_at = Instant::now() + RETRY;
                continue 'connect;
            }
        }
        loop {
            tokio::select! {
                msg = out.recv() => match msg {
                    Some(msg) => {
                        if ws.send(encode(msg)).await.is_err() {
                            // An utterance cut off by the drop is lost: the server only transcribes what it saw
                            // start and end
                            retry_at = Instant::now() + RETRY;
                            continue 'connect;
                        }
                    }
                    None => {
                        let _ = ws.close(None).await;
                        return;
                    }
                },
                incoming = ws.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(msg) = serde_json::from_str::<SpeechServerMessage>(&text) {
                            let _ = notices.send(SpeechEvent::Server(msg));
                        }
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => {
                        retry_at = Instant::now() + RETRY;
                        continue 'connect;
                    }
                },
            }
        }
    }
}

fn size(msg: &Out) -> usize {
    match msg {
        Out::Audio(pcm) => pcm.len() * 2,
        _ => 32,
    }
}

fn encode(msg: Out) -> Message {
    let json = |m: SpeechClientMessage| Message::text(serde_json::to_string(&m).unwrap_or_default());
    match msg {
        Out::Start(lang) => json(SpeechClientMessage::Start { lang, live: None }),
        Out::End => json(SpeechClientMessage::End),
        Out::Cancel => json(SpeechClientMessage::Cancel),
        // 16-bit little-endian, what the server reads
        Out::Audio(pcm) => Message::binary(pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<u8>>()),
    }
}
