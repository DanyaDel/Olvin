//! What the game replicates and sends, by name. Names and registration order are
//! the protocol: the handshake compares a hash of them, so two builds that
//! disagree refuse to talk instead of misreading each other's bytes.

use crate::replicate::ErasedComponent;

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
    /// Replicated component types; the index is the id on the wire.
    pub(crate) components: Vec<Box<dyn ErasedComponent>>,
}

impl NetRegistry {
    /// Hash of the registered names, compared in the handshake.
    pub fn signature(&self) -> u64 {
        fnv1a(self.names.iter())
    }
}

impl NetRegistry {
    pub(crate) fn add_name(&mut self, kind: &str, name: &str) {
        assert!(
            !self.frozen,
            "net: `{name}` is registered after the session started. Register components and \
             messages in your plugin's build(), before the first frame."
        );
        let full = format!("{kind}:{name}");
        assert!(
            !self.names.contains(&full),
            "net: {kind} `{name}` is registered twice"
        );
        self.names.push(full);
    }

    /// Names of the replicated components, in wire order.
    pub fn component_names(&self) -> impl Iterator<Item = &str> {
        self.components.iter().map(|c| c.name())
    }
}
