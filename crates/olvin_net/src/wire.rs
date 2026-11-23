//! Wire format. Packets are `postcard` (serde) bodies behind a two-byte header
//! `[MAGIC][kind]`. Everything that comes from a peer goes through [`decode`],
//! which checks the size first and refuses trailing bytes; nothing here panics
//! on hostile input.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// First byte of every packet: lets a stray UDP packet or a wrong-protocol peer be told apart.
pub const MAGIC: u8 = 0xB7;
/// Protocol revision. It is the first field of `Hello`, and `Hello` is the one
/// packet whose layout never changes, so a server can always say "wrong version".
pub const PROTOCOL_VERSION: u16 = 1;

/// Packet kinds (the second header byte).
pub mod kind {
    pub const HELLO: u8 = 1;
    pub const WELCOME: u8 = 2;
    pub const REJECT: u8 = 3;
    /// Sealed session data: `seq u64 LE | tag [u8; 8] | postcard(Msg)`.
    pub const DATA: u8 = 4;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WireError {
    TooLarge { len: usize, max: usize },
    Truncated,
    BadMagic,
    BadKind(u8),
    Decode(String),
    TrailingBytes,
    Encode(String),
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge { len, max } => {
                write!(f, "packet of {len} bytes exceeds the {max} limit")
            }
            Self::Truncated => f.write_str("packet is too short"),
            Self::BadMagic => f.write_str("not an Olvin packet (bad magic byte)"),
            Self::BadKind(k) => write!(f, "unknown packet kind {k}"),
            Self::Decode(e) => write!(f, "malformed packet: {e}"),
            Self::TrailingBytes => f.write_str("malformed packet: trailing bytes"),
            Self::Encode(e) => write!(f, "cannot encode: {e}"),
        }
    }
}

impl std::error::Error for WireError {}

/// Serialises `value`.
pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, WireError> {
    postcard::to_stdvec(value).map_err(|e| WireError::Encode(e.to_string()))
}

/// Parses `bytes` as exactly one `T`. `max` bounds the input *before* any parsing.
pub fn decode<T: DeserializeOwned>(bytes: &[u8], max: usize) -> Result<T, WireError> {
    if bytes.len() > max {
        return Err(WireError::TooLarge {
            len: bytes.len(),
            max,
        });
    }
    let (value, rest) =
        postcard::take_from_bytes::<T>(bytes).map_err(|e| WireError::Decode(e.to_string()))?;
    if !rest.is_empty() {
        return Err(WireError::TrailingBytes);
    }
    Ok(value)
}

/// `[MAGIC][kind][body...]`.
pub fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 2);
    out.push(MAGIC);
    out.push(kind);
    out.extend_from_slice(body);
    out
}

/// Splits a packet into its kind and body.
pub fn unframe(bytes: &[u8]) -> Result<(u8, &[u8]), WireError> {
    match bytes {
        [] | [_] => Err(WireError::Truncated),
        [m, ..] if *m != MAGIC => Err(WireError::BadMagic),
        [_, k, body @ ..] => Ok((*k, body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_strictness() {
        let bytes = encode(&(1u32, "hi".to_string())).unwrap();
        assert_eq!(
            decode::<(u32, String)>(&bytes, 64).unwrap(),
            (1, "hi".into())
        );
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(
            decode::<(u32, String)>(&extra, 64),
            Err(WireError::TrailingBytes)
        );
        assert!(matches!(
            decode::<(u32, String)>(&bytes, 1),
            Err(WireError::TooLarge { .. })
        ));
    }

    #[test]
    fn hostile_length_prefix_does_not_allocate_or_panic() {
        // A Vec<u64> claiming u64::MAX elements.
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x01];
        assert!(decode::<Vec<u64>>(&bytes, 64).is_err());
        assert!(decode::<String>(&bytes, 64).is_err());
    }

    #[test]
    fn frames() {
        assert_eq!(unframe(&[]), Err(WireError::Truncated));
        assert_eq!(unframe(&[0, 1, 2]), Err(WireError::BadMagic));
        assert_eq!(unframe(&frame(4, &[9])).unwrap(), (4, &[9][..]));
    }
}
