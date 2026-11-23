//! Sealing and opening session packets: sequence number, tag, replay window.

use crate::crypto::{Direction, ReplayWindow, SessionKey, TAG_LEN};
use crate::proto::Msg;
use crate::transport::Channel;
use crate::wire::{self, WireError, kind};

#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Shorter than the fixed header.
    Short,
    /// The tag does not match: corrupted, forged or from another session.
    BadTag,
    /// A valid packet seen before, or too old: dropped silently.
    Replay,
    Malformed(WireError),
}

/// One direction pair of an established session.
pub struct Secure {
    key: SessionKey,
    send_dir: Direction,
    send_seq: u64,
    replay: ReplayWindow,
}

impl Secure {
    pub fn new(key: SessionKey, send_dir: Direction) -> Self {
        Self {
            key,
            send_dir,
            send_seq: 0,
            replay: ReplayWindow::default(),
        }
    }

    fn recv_dir(&self) -> Direction {
        match self.send_dir {
            Direction::ClientToServer => Direction::ServerToClient,
            Direction::ServerToClient => Direction::ClientToServer,
        }
    }

    /// `[MAGIC][DATA] seq tag body` for an encoded message body.
    pub fn seal_bytes(&mut self, channel: Channel, body: &[u8]) -> Vec<u8> {
        self.send_seq += 1;
        let seq = self.send_seq;
        let tag = self.key.tag(self.send_dir, channel, seq, body);
        let mut out = Vec::with_capacity(body.len() + 8 + TAG_LEN + 2);
        out.push(wire::MAGIC);
        out.push(kind::DATA);
        out.extend_from_slice(&seq.to_le_bytes());
        out.extend_from_slice(&tag);
        out.extend_from_slice(body);
        out
    }

    pub fn seal(&mut self, channel: Channel, msg: &Msg) -> Option<Vec<u8>> {
        let body = wire::encode(msg).ok()?;
        Some(self.seal_bytes(channel, &body))
    }

    /// Verifies and unwraps the part of a `DATA` packet after the two header bytes.
    pub fn open_bytes<'a>(
        &mut self,
        channel: Channel,
        packet: &'a [u8],
    ) -> Result<&'a [u8], OpenError> {
        if packet.len() < 8 + TAG_LEN {
            return Err(OpenError::Short);
        }
        let (seq, rest) = packet.split_at(8);
        let (tag, body) = rest.split_at(TAG_LEN);
        let mut seq_bytes = [0u8; 8];
        seq_bytes.copy_from_slice(seq);
        let seq = u64::from_le_bytes(seq_bytes);
        let mut tag_bytes = [0u8; TAG_LEN];
        tag_bytes.copy_from_slice(tag);
        if !self
            .key
            .verify(self.recv_dir(), channel, seq, body, &tag_bytes)
        {
            return Err(OpenError::BadTag);
        }
        if !self.replay.accept(seq) {
            return Err(OpenError::Replay);
        }
        Ok(body)
    }

    pub fn open(&mut self, channel: Channel, packet: &[u8], max: usize) -> Result<Msg, OpenError> {
        let body = self.open_bytes(channel, packet)?;
        wire::decode(body, max).map_err(OpenError::Malformed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Secure, Secure) {
        let key = SessionKey::derive(b"s", &[1; 16], &[2; 16]);
        (
            Secure::new(key.clone(), Direction::ClientToServer),
            Secure::new(key, Direction::ServerToClient),
        )
    }

    #[test]
    fn roundtrip_replay_and_tamper() {
        let (mut client, mut server) = pair();
        let msg = Msg::Ping { id: 3, sent: 1.5 };
        let packet = client.seal(Channel::Unreliable, &msg).unwrap();
        let (k, body) = wire::unframe(&packet).unwrap();
        assert_eq!(k, kind::DATA);
        assert_eq!(
            server.open(Channel::Unreliable, body, 1024),
            Ok(msg.clone())
        );
        // The same bytes again are a replay.
        assert_eq!(
            server.open(Channel::Unreliable, body, 1024),
            Err(OpenError::Replay)
        );
        // Flip a body bit.
        let packet = client.seal(Channel::Unreliable, &msg).unwrap();
        let (_, body) = wire::unframe(&packet).unwrap();
        let mut bad = body.to_vec();
        *bad.last_mut().unwrap() ^= 1;
        assert_eq!(
            server.open(Channel::Unreliable, &bad, 1024),
            Err(OpenError::BadTag)
        );
        // Wrong channel.
        assert_eq!(
            server.open(Channel::Reliable, body, 1024),
            Err(OpenError::BadTag)
        );
        // A packet sent to the server cannot be bounced back at the client.
        assert_eq!(
            client.open(Channel::Unreliable, body, 1024),
            Err(OpenError::BadTag)
        );
        assert_eq!(
            server.open(Channel::Unreliable, &[1, 2], 64),
            Err(OpenError::Short)
        );
    }
}
