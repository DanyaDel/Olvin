//! Signed save files (ROADMAP phase 8, S1).
//!
//! A save carries an HMAC-SHA256 of its text as the last line, a RON comment:
//! `// olvin-save-hmac: <hex>`. The file stays one plain RON file (written
//! atomically like before), and saves without the line still load.
//!
//! What this gives, honestly: **a signal and a deterrent.** The key is part of
//! the game, so someone who takes it out of the binary can sign their own
//! edits. It stops casual tampering with a text editor and tells the game when
//! a save was edited; it is not protection against a determined cheater.

use std::fmt;
use std::sync::Arc;

use hmac::{Hmac, Mac};
use olvin_core::secret::SecretBytes;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Starts the signature line. A leading newline keeps it off the data's last line.
const MARKER: &str = "\n// olvin-save-hmac: ";

/// What to do with a save whose signature does not match.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SavePolicy {
    /// Load it and log a warning (the default: the game decides what to do).
    #[default]
    Warn,
    /// Refuse a save whose signature does not match. Saves without a
    /// signature (made before signing was turned on) still load.
    Enforce,
    /// Refuse a mismatching save **and** a save without a signature. For games
    /// that never wrote unsigned saves; removing the line cannot bypass it.
    Strict,
}

/// How a loaded save relates to its signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveStatus {
    /// No key is configured, so nothing was checked.
    Unchecked,
    /// A key is configured and the file has no signature line.
    Unsigned,
    /// The signature matches the text.
    Valid,
    /// The text or the signature was changed (or another key signed it).
    Tampered,
}

/// The signing key and the policy for mismatches.
#[derive(Clone)]
pub struct SaveSigner {
    key: Arc<SecretBytes>,
    pub policy: SavePolicy,
}

impl SaveSigner {
    /// `key` is any non-empty secret; the game embeds it (see the module docs).
    pub fn new(key: SecretBytes) -> Self {
        Self {
            key: Arc::new(key),
            policy: SavePolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: SavePolicy) -> Self {
        self.policy = policy;
        self
    }

    fn mac(&self, body: &str) -> Option<HmacSha256> {
        let mut mac = HmacSha256::new_from_slice(self.key.expose()).ok()?;
        mac.update(body.as_bytes());
        Some(mac)
    }

    /// `text` plus the signature line. `None` only for an empty key.
    pub fn sign(&self, text: &str) -> Option<String> {
        let tag = self.mac(text)?.finalize().into_bytes();
        let mut out = String::with_capacity(text.len() + MARKER.len() + 65);
        out.push_str(text);
        out.push_str(MARKER);
        for byte in tag {
            out.push_str(&format!("{byte:02x}"));
        }
        out.push('\n');
        Some(out)
    }

    /// Checks `text` (a whole save file).
    pub fn check(&self, text: &str) -> SaveStatus {
        let Some(at) = text.rfind(MARKER) else {
            return SaveStatus::Unsigned;
        };
        let (body, tail) = (&text[..at], &text[at + MARKER.len()..]);
        let Some(given) = decode_hex(tail.trim()) else {
            return SaveStatus::Tampered;
        };
        // `verify_slice` compares in constant time.
        let valid = self
            .mac(body)
            .is_some_and(|mac| mac.verify_slice(&given).is_ok());
        if valid {
            SaveStatus::Valid
        } else {
            SaveStatus::Tampered
        }
    }

    /// Whether `status` may load under the policy.
    pub fn allows(&self, status: SaveStatus) -> bool {
        match status {
            SaveStatus::Unchecked | SaveStatus::Valid => true,
            SaveStatus::Unsigned => self.policy != SavePolicy::Strict,
            SaveStatus::Tampered => self.policy == SavePolicy::Warn,
        }
    }
}

impl fmt::Debug for SaveSigner {
    // Never the key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SaveSigner")
            .field("key", &"<hidden>")
            .field("policy", &self.policy)
            .finish()
    }
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

/// The file's text without the signature line, for the RON parser (which would
/// accept the comment anyway; this keeps the parsed text exactly what was signed).
pub fn strip_signature(text: &str) -> &str {
    text.rfind(MARKER).map_or(text, |at| &text[..at])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signer(policy: SavePolicy) -> SaveSigner {
        SaveSigner::new(SecretBytes::new(b"test key".to_vec())).with_policy(policy)
    }

    #[test]
    fn signed_text_checks_out_and_edits_do_not() {
        let s = signer(SavePolicy::Warn);
        let signed = s.sign("(version: 1, gold: 10)").unwrap();
        assert_eq!(s.check(&signed), SaveStatus::Valid);
        assert_eq!(strip_signature(&signed), "(version: 1, gold: 10)");
        let edited = signed.replace("gold: 10", "gold: 99");
        assert_eq!(s.check(&edited), SaveStatus::Tampered);
    }

    #[test]
    fn another_key_is_a_mismatch() {
        let signed = signer(SavePolicy::Warn).sign("(a: 1)").unwrap();
        let other = SaveSigner::new(SecretBytes::new(b"other".to_vec()));
        assert_eq!(other.check(&signed), SaveStatus::Tampered);
    }

    #[test]
    fn a_missing_line_is_unsigned_and_garbage_is_tampered() {
        let s = signer(SavePolicy::Warn);
        assert_eq!(s.check("(a: 1)"), SaveStatus::Unsigned);
        assert_eq!(
            s.check(&format!("(a: 1){MARKER}not hex at all\n")),
            SaveStatus::Tampered
        );
        assert_eq!(
            s.check(&format!("(a: 1){MARKER}abc\n")),
            SaveStatus::Tampered
        );
    }

    #[test]
    fn policies_decide_what_loads() {
        let warn = signer(SavePolicy::Warn);
        let enforce = signer(SavePolicy::Enforce);
        let strict = signer(SavePolicy::Strict);
        for (status, w, e, st) in [
            (SaveStatus::Valid, true, true, true),
            (SaveStatus::Unsigned, true, true, false),
            (SaveStatus::Tampered, true, false, false),
            (SaveStatus::Unchecked, true, true, true),
        ] {
            assert_eq!(warn.allows(status), w, "{status:?} under Warn");
            assert_eq!(enforce.allows(status), e, "{status:?} under Enforce");
            assert_eq!(strict.allows(status), st, "{status:?} under Strict");
        }
    }

    #[test]
    fn debug_never_prints_the_key() {
        let text = format!("{:?}", signer(SavePolicy::Warn));
        assert!(!text.contains("test key") && text.contains("<hidden>"));
    }
}
