//! What arrives over `/ws/user`: the lists, and rings both ways.

use protocol::{CallEndReason, UserServerMessage};

use super::socket::SocketEvent;
use super::{Controller, FriendsEvent, IncomingCall};

impl Controller {
    pub(super) fn socket_event(&mut self, event: SocketEvent) {
        match event {
            SocketEvent::Open => self.state.connected = true,
            SocketEvent::Lost => {
                self.state.connected = false;
                self.state.outgoing = None;
                self.state.incoming.clear();
            }
            SocketEvent::SignedOut => self.signed_out(),
            SocketEvent::Message(msg) => self.message(msg),
        }
    }

    fn message(&mut self, msg: UserServerMessage) {
        match msg {
            UserServerMessage::State(lists) => self.state.lists = Some(lists),
            UserServerMessage::Ringing { call_id, to } => {
                if let Some(out) = &mut self.state.outgoing
                    && out.to == to
                {
                    out.call_id = Some(call_id);
                }
            }
            UserServerMessage::Incoming { call_id, from } => {
                if !self.state.incoming.iter().any(|c| c.call_id == call_id) {
                    self.state.incoming.push(IncomingCall { call_id, from });
                }
            }
            UserServerMessage::Answered { call_id, room_id } => {
                self.state.outgoing = None;
                self.state.incoming.retain(|c| c.call_id != call_id);
                if protocol::is_room_id(&room_id) {
                    (self.on_event)(FriendsEvent::Answered(room_id));
                }
            }
            UserServerMessage::CallEnded { call_id, reason } => {
                let mine =
                    self.state.outgoing.as_ref().is_some_and(|o| o.call_id.as_ref().is_none_or(|id| *id == call_id));
                if mine {
                    if let Some(out) = self.state.outgoing.take()
                        && reason != CallEndReason::Cancelled
                    {
                        self.state.notice = Some((reason, out.to));
                    }
                } else {
                    self.state.incoming.retain(|c| c.call_id != call_id);
                }
            }
            UserServerMessage::Pong | UserServerMessage::Unknown => {}
        }
    }
}
