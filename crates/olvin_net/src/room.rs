//! Room codes: one short string that carries the host's address *and* the
//! fingerprint of its certificate, so joining is "paste the code" and the
//! connection is pinned to the right server (see ADR 0002).
//!
//! `OLV-` followed by groups of five Crockford base32 characters
//! (`0-9 A-Z` without `I L O U`). Layout before encoding:
//! `version | family | addr (4 or 16) | port (BE) | fingerprint[..16] | checksum (2)`.
//! 128 bits of the certificate hash are enough to pin it: finding another key
//! with the same prefix is out of reach.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::str::FromStr;

use crate::crypto::sha256;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const VERSION: u8 = 1;
pub const PIN_LEN: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoomCode {
    pub addr: SocketAddr,
    /// First 16 bytes of the SHA-256 of the server certificate (DER).
    pub pin: [u8; PIN_LEN],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomCodeError {
    Empty,
    BadCharacter(char),
    BadLength,
    BadChecksum,
    BadVersion(u8),
}

impl fmt::Display for RoomCodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("the room code is empty"),
            Self::BadCharacter(c) => write!(f, "the room code has an invalid character {c:?}"),
            Self::BadLength => f.write_str("the room code has the wrong length (copy all of it)"),
            Self::BadChecksum => f.write_str("the room code has a typo (checksum mismatch)"),
            Self::BadVersion(v) => write!(f, "the room code is version {v}, this build reads 1"),
        }
    }
}

impl std::error::Error for RoomCodeError {}

impl RoomCode {
    /// The code for a server at `addr` whose certificate hashes to `cert_sha256`.
    pub fn new(addr: SocketAddr, cert_sha256: &[u8; 32]) -> Self {
        let mut pin = [0u8; PIN_LEN];
        pin.copy_from_slice(&cert_sha256[..PIN_LEN]);
        Self { addr, pin }
    }

    /// Whether a certificate with this SHA-256 is the one the code pins.
    pub fn pins(&self, cert_sha256: &[u8; 32]) -> bool {
        crate::crypto::ct_eq(&self.pin, &cert_sha256[..PIN_LEN])
    }

    fn bytes(&self) -> Vec<u8> {
        let mut b = vec![VERSION];
        match self.addr.ip() {
            IpAddr::V4(ip) => {
                b.push(4);
                b.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                b.push(6);
                b.extend_from_slice(&ip.octets());
            }
        }
        b.extend_from_slice(&self.addr.port().to_be_bytes());
        b.extend_from_slice(&self.pin);
        let sum = sha256(&b);
        b.extend_from_slice(&sum[..2]);
        b
    }
}

impl fmt::Display for RoomCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = base32(&self.bytes());
        f.write_str("OLV")?;
        for chunk in text.as_bytes().chunks(5) {
            f.write_str("-")?;
            f.write_str(std::str::from_utf8(chunk).unwrap_or(""))?;
        }
        Ok(())
    }
}

impl FromStr for RoomCode {
    type Err = RoomCodeError;

    /// Lenient: any case, dashes and spaces ignored, `O` read as `0`, `I` and `L` as `1`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.is_empty() {
            return Err(RoomCodeError::Empty);
        }
        if s.len() > 128 {
            return Err(RoomCodeError::BadLength);
        }
        let upper = s.to_ascii_uppercase();
        let body = upper.strip_prefix("OLV").unwrap_or(&upper);
        let mut values = Vec::new();
        for c in body.chars().filter(|c| !matches!(c, '-' | ' ' | '_')) {
            let c = match c {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let v = ALPHABET
                .iter()
                .position(|a| *a as char == c)
                .ok_or(RoomCodeError::BadCharacter(c))?;
            values.push(v as u8);
        }
        let bytes = unbase32(&values);
        let (&version, rest) = bytes.split_first().ok_or(RoomCodeError::BadLength)?;
        if version != VERSION {
            return Err(RoomCodeError::BadVersion(version));
        }
        let (&family, rest) = rest.split_first().ok_or(RoomCodeError::BadLength)?;
        let addr_len = match family {
            4 => 4,
            6 => 16,
            _ => return Err(RoomCodeError::BadLength),
        };
        // addr, port, pin, checksum; trailing bits of the last character may add
        // up to one zero byte at most.
        let need = addr_len + 2 + PIN_LEN + 2;
        if rest.len() < need || rest.len() > need + 1 || rest[need..].iter().any(|b| *b != 0) {
            return Err(RoomCodeError::BadLength);
        }
        let (addr, rest) = rest.split_at(addr_len);
        let (port, rest) = rest.split_at(2);
        let (pin, rest) = rest.split_at(PIN_LEN);
        let sum = &rest[..2];
        let ip = match <[u8; 4]>::try_from(addr) {
            Ok(o) => IpAddr::V4(Ipv4Addr::from(o)),
            Err(_) => IpAddr::V6(Ipv6Addr::from(
                <[u8; 16]>::try_from(addr).map_err(|_| RoomCodeError::BadLength)?,
            )),
        };
        let port = u16::from_be_bytes([port[0], port[1]]);
        let mut pin_arr = [0u8; PIN_LEN];
        pin_arr.copy_from_slice(pin);
        let code = RoomCode {
            addr: SocketAddr::new(ip, port),
            pin: pin_arr,
        };
        let expect = sha256(&code.bytes()[..code.bytes().len() - 2]);
        if sum != &expect[..2] {
            return Err(RoomCodeError::BadChecksum);
        }
        Ok(code)
    }
}

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &b in bytes {
        acc = (acc << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn unbase32(values: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0);
    for &v in values {
        acc = (acc << 5) | v as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 0xFF) as u8);
        }
    }
    out
}

/// What a client joins: a room code or a plain address.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinTarget {
    Room(RoomCode),
    /// `host:port`, no certificate pinning (refused unless `allow_unpinned`).
    Addr(String),
}

impl JoinTarget {
    /// A room code if the text parses as one, otherwise an address.
    pub fn parse(text: &str) -> Result<Self, String> {
        let t = text.trim();
        if t.to_ascii_uppercase().starts_with("OLV") {
            return t
                .parse::<RoomCode>()
                .map(Self::Room)
                .map_err(|e| e.to_string());
        }
        if t.is_empty() {
            return Err("nothing to join: give a room code (OLV-...) or host:port".into());
        }
        if !t.contains(':') {
            return Err(format!(
                "`{t}` is neither a room code (OLV-...) nor host:port"
            ));
        }
        Ok(Self::Addr(t.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(addr: &str) -> RoomCode {
        RoomCode::new(addr.parse().unwrap(), &sha256(b"cert"))
    }

    #[test]
    fn roundtrip_v4_and_v6() {
        for a in ["192.168.1.20:7777", "[::1]:4433", "127.0.0.1:1"] {
            let code = sample(a);
            let text = code.to_string();
            assert!(text.starts_with("OLV-"), "{text}");
            assert_eq!(text.parse::<RoomCode>().unwrap(), code, "{text}");
            assert!(code.pins(&sha256(b"cert")));
            assert!(!code.pins(&sha256(b"other")));
        }
    }

    #[test]
    fn lenient_input_and_typos_are_caught() {
        let code = sample("10.0.0.5:9000");
        let text = code.to_string();
        let sloppy = text.to_lowercase().replace('-', " ");
        assert_eq!(sloppy.parse::<RoomCode>().unwrap(), code);
        // Flip one character.
        let mut chars: Vec<char> = text.chars().collect();
        let i = chars.len() - 3;
        chars[i] = if chars[i] == '7' { '8' } else { '7' };
        let bad: String = chars.into_iter().collect();
        assert!(bad.parse::<RoomCode>().is_err());
    }

    #[test]
    fn garbage_never_panics() {
        for s in [
            "",
            "OLV",
            "OLV-",
            "OLV-U",
            "!!!",
            "OLV-0000",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            &"9".repeat(500),
        ] {
            assert!(s.parse::<RoomCode>().is_err(), "{s:?}");
        }
        let mut x = 12345u64;
        for _ in 0..2000 {
            let len = (x % 60) as usize;
            let s: String = (0..len)
                .map(|_| {
                    x = x
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    ALPHABET[((x >> 59) & 31) as usize] as char
                })
                .collect();
            let _ = s.parse::<RoomCode>();
        }
    }

    #[test]
    fn join_target_parse() {
        assert!(matches!(
            JoinTarget::parse("127.0.0.1:7777"),
            Ok(JoinTarget::Addr(_))
        ));
        assert!(matches!(
            JoinTarget::parse(&sample("1.2.3.4:5").to_string()),
            Ok(JoinTarget::Room(_))
        ));
        assert!(JoinTarget::parse("hello").is_err());
        assert!(JoinTarget::parse("").is_err());
    }
}
