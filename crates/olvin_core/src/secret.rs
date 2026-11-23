//! Handling of secrets in memory and in the binary (roadmap E1, E3).
//!
//! * [`SecretString`] and [`SecretBytes`] own a secret (a password, a token, a
//!   signing key), overwrite it with zeros when dropped, and never print it:
//!   their `Debug` shows only that something is hidden, there is no `Display`,
//!   and they are not `Clone` (a copy is another place to forget to wipe).
//!   Reading is explicit: [`SecretString::expose`], [`SecretBytes::expose`].
//! * [`obf!`] keeps a string literal out of the binary's `.rodata` in cleartext.
//!
//! What this gives, honestly. Zeroizing shortens the time a secret sits in
//! memory after use, so a dump taken later does not find it; it does not hide
//! it from someone who can read the process while it is in use, and it cannot
//! reach copies the secret was already copied into (a `String` you built
//! before wrapping it, a terminal's scrollback, swap). `obf!` raises the cost of
//! `strings game.exe` from zero to a few minutes of reverse engineering: the
//! literal is rebuilt at run time, so a debugger or a memory dump sees it. It is
//! deterrence, not protection, and must never be the only thing guarding a
//! secret that matters.

use std::fmt;

use zeroize::{Zeroize, Zeroizing};

#[doc(hidden)]
pub use obfstr as __obfstr;

/// A string literal that is not stored as such in the binary: the bytes are
/// kept scrambled and an owned `String` is rebuilt where the macro runs. The
/// scrambling key is picked at compile time from the literal's source position.
///
/// ```
/// let url = olvin_core::secret::obf!("https://example.invalid/license");
/// assert_eq!(url, "https://example.invalid/license");
/// ```
///
/// Use it for the few literals that point at something sensitive: a licence or
/// validation server, a check's error text, a key's name. Not for every string:
/// each use costs a small decode at run time. Deterrence only; see the
/// [module docs](self).
#[macro_export]
#[doc(hidden)]
macro_rules! __obf {
    ($text:literal) => {
        $crate::secret::__obfstr::obfstring!($text)
    };
}

#[doc(inline)]
pub use __obf as obf;

/// A secret text, wiped on drop. See the [module docs](self).
pub struct SecretString(Zeroizing<String>);

impl SecretString {
    /// Takes ownership: the text now lives only here (nothing is copied).
    pub fn new(text: String) -> Self {
        Self(Zeroizing::new(text))
    }

    /// The secret, for the moment it is needed. Do not store the result.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Wipes the text now instead of at drop.
    pub fn wipe(&mut self) {
        self.0.zeroize();
    }

    /// Compares in time that does not depend on where the texts first differ.
    pub fn ct_eq(&self, other: &SecretString) -> bool {
        ct_eq(self.0.as_bytes(), other.0.as_bytes())
    }
}

impl From<String> for SecretString {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

impl fmt::Debug for SecretString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretString([hidden])")
    }
}

/// A secret byte string (a raw key), wiped on drop. See the [module docs](self).
pub struct SecretBytes(Zeroizing<Vec<u8>>);

impl SecretBytes {
    /// Takes ownership: the bytes now live only here (nothing is copied).
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }

    /// The secret, for the moment it is needed. Do not store the result.
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Wipes the bytes now (the buffer is emptied) instead of at drop.
    pub fn wipe(&mut self) {
        self.0.zeroize();
    }

    /// Compares in time that does not depend on where the bytes first differ.
    pub fn ct_eq(&self, other: &SecretBytes) -> bool {
        ct_eq(&self.0, &other.0)
    }
}

impl From<Vec<u8>> for SecretBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretBytes([hidden])")
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b) {
        diff |= x ^ y;
    }
    std::hint::black_box(diff) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_shows_the_secret() {
        let text = SecretString::new("hunter2-token".into());
        let bytes = SecretBytes::new(b"hunter2-key".to_vec());
        for shown in [format!("{text:?}"), format!("{bytes:?}")] {
            assert!(!shown.contains("hunter2"), "{shown}");
            assert!(shown.contains("hidden"));
        }
        // Nested in another type's derived Debug it stays hidden too.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Login {
            user: String,
            password: SecretString,
        }
        let login = Login {
            user: "dan".into(),
            password: SecretString::new("hunter2".into()),
        };
        assert!(!format!("{login:?}").contains("hunter2"));
    }

    #[test]
    fn expose_and_wipe() {
        let mut text = SecretString::from("abc".to_owned());
        assert_eq!(text.expose(), "abc");
        assert_eq!(text.len(), 3);
        text.wipe();
        assert!(text.is_empty());
        let mut bytes = SecretBytes::from(vec![1, 2, 3]);
        assert_eq!(bytes.expose(), [1, 2, 3]);
        bytes.wipe();
        assert!(bytes.is_empty());
    }

    #[test]
    fn wipe_overwrites_the_buffer_in_place() {
        let mut bytes = SecretBytes::new(vec![0xAA; 32]);
        let ptr = bytes.expose().as_ptr();
        bytes.wipe();
        // SAFETY: `wipe` clears the vector but keeps its allocation (capacity 32),
        // so the 32 bytes behind `ptr` are still owned by `bytes`.
        let after = unsafe { std::slice::from_raw_parts(ptr, 32) };
        assert!(after.iter().all(|b| *b == 0), "{after:?}");
    }

    #[test]
    fn constant_time_comparison() {
        assert!(SecretString::new("same".into()).ct_eq(&SecretString::new("same".into())));
        assert!(!SecretString::new("same".into()).ct_eq(&SecretString::new("sane".into())));
        assert!(!SecretBytes::new(vec![1]).ct_eq(&SecretBytes::new(vec![1, 2])));
    }

    #[test]
    fn obf_returns_the_literal() {
        let text = obf!("a literal that round-trips");
        assert_eq!(text, "a literal that round-trips");
        assert_eq!(obf!(""), "");
        let unicode = obf!("ключ-лицензии 🔑");
        assert_eq!(unicode, "ключ-лицензии 🔑");
    }
}
