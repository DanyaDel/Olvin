//! Session keys, packet tags and replay protection (roadmap N2 and N3).
//!
//! Every sealed packet carries a strictly new sequence number and a truncated
//! HMAC-SHA256 tag over `(direction, channel, seq, body)`. The key is derived
//! from the two handshake nonces and, when the transport has one, a secret
//! that never travels on the wire (the TLS exporter on QUIC). Without that
//! secret the tag only guards against corruption, bugs and replays, not
//! against somebody who can read the wire: only an in-process transport goes
//! without one.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

use crate::transport::Channel;

type HmacSha256 = Hmac<Sha256>;

pub const NONCE_LEN: usize = 16;
pub const TAG_LEN: usize = 8;
pub type Nonce = [u8; NONCE_LEN];

/// Random bytes from the OS. `None` only if the OS refuses, which callers
/// treat as "cannot connect securely" rather than falling back to weak bytes.
pub fn random<const N: usize>() -> Option<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).ok()?;
    Some(out)
}

/// Which way a sealed packet travels; part of the MAC so a packet cannot be
/// bounced back at its sender.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}

#[derive(Clone)]
pub struct SessionKey([u8; 32]);

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey(..)")
    }
}

impl SessionKey {
    /// `HMAC(secret, "olvin-net v1" || client_nonce || server_nonce)`.
    pub fn derive(secret: &[u8], client_nonce: &Nonce, server_nonce: &Nonce) -> Self {
        let mut mac = keyed(secret);
        mac.update(b"olvin-net v1");
        mac.update(client_nonce);
        mac.update(server_nonce);
        Self(mac.finalize().into_bytes().into())
    }

    fn mac(&self, dir: Direction, channel: Channel, seq: u64) -> HmacSha256 {
        let mut mac = keyed(&self.0);
        mac.update(&[
            match dir {
                Direction::ClientToServer => 0,
                Direction::ServerToClient => 1,
            },
            match channel {
                Channel::Reliable => 0,
                Channel::Unreliable => 1,
            },
        ]);
        mac.update(&seq.to_le_bytes());
        mac
    }

    pub fn tag(&self, dir: Direction, channel: Channel, seq: u64, body: &[u8]) -> [u8; TAG_LEN] {
        let mut mac = self.mac(dir, channel, seq);
        mac.update(body);
        let full = mac.finalize().into_bytes();
        let mut out = [0u8; TAG_LEN];
        out.copy_from_slice(&full[..TAG_LEN]);
        out
    }

    /// Constant-time check of a received tag.
    pub fn verify(
        &self,
        dir: Direction,
        channel: Channel,
        seq: u64,
        body: &[u8],
        tag: &[u8; TAG_LEN],
    ) -> bool {
        let mut mac = self.mac(dir, channel, seq);
        mac.update(body);
        mac.verify_truncated_left(tag).is_ok()
    }

    /// Proof of the key for the `Welcome` packet.
    pub fn proof(&self, label: &[u8]) -> [u8; 16] {
        let mut mac = keyed(&self.0);
        mac.update(b"proof");
        mac.update(label);
        let full = mac.finalize().into_bytes();
        let mut out = [0u8; 16];
        out.copy_from_slice(&full[..16]);
        out
    }
}

fn keyed(key: &[u8]) -> HmacSha256 {
    // HMAC accepts any key length; a fixed block-sized key keeps this infallible.
    let mut block = [0u8; 64];
    if key.len() <= block.len() {
        block[..key.len()].copy_from_slice(key);
    } else {
        block[..32].copy_from_slice(&Sha256::digest(key));
    }
    HmacSha256::new(&block.into())
}

/// Constant-time equality for secrets (tokens, passwords).
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    // Compare digests so the length does not leak and the loop is fixed.
    let (ha, hb) = (Sha256::digest(a), Sha256::digest(b));
    let mut diff = 0u8;
    for (x, y) in ha.iter().zip(hb.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// SHA-256 of `data` (certificate fingerprints, room codes).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Sliding window of accepted sequence numbers: rejects duplicates and
/// anything older than 128 behind the newest.
#[derive(Clone, Debug, Default)]
pub struct ReplayWindow {
    highest: u64,
    seen: u128,
    any: bool,
}

impl ReplayWindow {
    /// Records `seq` and returns whether it is new. Call it only after the tag verified.
    pub fn accept(&mut self, seq: u64) -> bool {
        if !self.any {
            self.any = true;
            self.highest = seq;
            self.seen = 1;
            return true;
        }
        if seq > self.highest {
            let shift = seq - self.highest;
            self.seen = if shift >= 128 { 0 } else { self.seen << shift };
            self.seen |= 1;
            self.highest = seq;
            return true;
        }
        let back = self.highest - seq;
        if back >= 128 {
            return false;
        }
        let bit = 1u128 << back;
        if self.seen & bit != 0 {
            return false;
        }
        self.seen |= bit;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_bind_direction_channel_seq_and_body() {
        let key = SessionKey::derive(b"secret", &[1; 16], &[2; 16]);
        let tag = key.tag(Direction::ClientToServer, Channel::Reliable, 5, b"abc");
        assert!(key.verify(
            Direction::ClientToServer,
            Channel::Reliable,
            5,
            b"abc",
            &tag
        ));
        assert!(!key.verify(
            Direction::ServerToClient,
            Channel::Reliable,
            5,
            b"abc",
            &tag
        ));
        assert!(!key.verify(
            Direction::ClientToServer,
            Channel::Unreliable,
            5,
            b"abc",
            &tag
        ));
        assert!(!key.verify(
            Direction::ClientToServer,
            Channel::Reliable,
            6,
            b"abc",
            &tag
        ));
        assert!(!key.verify(
            Direction::ClientToServer,
            Channel::Reliable,
            5,
            b"abd",
            &tag
        ));
        let other = SessionKey::derive(b"other", &[1; 16], &[2; 16]);
        assert!(!other.verify(
            Direction::ClientToServer,
            Channel::Reliable,
            5,
            b"abc",
            &tag
        ));
    }

    #[test]
    fn replay_window() {
        let mut w = ReplayWindow::default();
        assert!(w.accept(10));
        assert!(!w.accept(10));
        assert!(w.accept(8));
        assert!(!w.accept(8));
        assert!(w.accept(11));
        assert!(w.accept(500));
        // Too old now.
        assert!(!w.accept(10));
        assert!(w.accept(499));
        assert!(!w.accept(499));
        assert!(w.accept(u64::MAX));
        assert!(!w.accept(u64::MAX));
    }

    #[test]
    fn constant_time_eq_and_random() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abcd"));
        let (a, b) = (random::<16>().unwrap(), random::<16>().unwrap());
        assert_ne!(a, b);
    }
}
