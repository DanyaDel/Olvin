//! What the game replicates and sends, by name. Names and registration order are
//! the protocol: the handshake compares a hash of them, so two builds that
//! disagree refuse to talk instead of misreading each other's bytes.

/// FNV-1a, 64 bit: stable across platforms and releases (unlike `DefaultHasher`).
pub(crate) fn fnv1a(parts: impl IntoIterator<Item = impl AsRef<[u8]>>) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in part.as_ref().iter().chain(&[0xFFu8]) {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Resource part of [`Net`](crate::Net): the registrations. Grows with the layers above.
#[derive(Default)]
pub struct NetRegistry {
    /// `kind:name` of everything registered, in order.
    pub(crate) names: Vec<String>,
    pub(crate) frozen: bool,
}

impl NetRegistry {
    /// Hash of the registered names, compared in the handshake.
    pub fn signature(&self) -> u64 {
        fnv1a(self.names.iter())
    }
}
