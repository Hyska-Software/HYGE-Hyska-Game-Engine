//! Content-addressed asset identity via BLAKE3.
//!
//! Every asset in Hyge is identified by the BLAKE3 hash of its source
//! bytes; two files with identical content have the same [`AssetId`]
//! regardless of where they live on disk. See ADR-0006.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A BLAKE3-hashed asset identity: 32 bytes, `Copy`, `Eq`, `Hash`,
/// `Serialize`/`Deserialize` (as a 64-character hex string).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AssetId(#[serde(with = "hex_serde")] pub [u8; 32]);

impl AssetId {
    /// The all-zeros sentinel, conventionally used for "no asset".
    pub const NULL: AssetId = AssetId([0u8; 32]);

    /// Constructs an `AssetId` from the 32 raw hash bytes.
    #[inline]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the 32 raw hash bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns true if this is the null sentinel (all zero bytes).
    #[inline]
    pub const fn is_null(&self) -> bool {
        // Const-friendly check: every byte must be zero.
        let mut i = 0;
        while i < 32 {
            if self.0[i] != 0 {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Returns the lowercase hex representation (no `0x` prefix, 64 chars).
    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for b in &self.0 {
            // `format!` is the most readable approach and is fast enough
            // for the ~1B/s hashing workloads this is used in.
            s.push_str(&format!("{b:02x}"));
        }
        s
    }

    /// Parses a 64-character hex string. An optional `0x` prefix is
    /// accepted. Returns [`crate::result::HygeError::Parse`] on length mismatch or
    /// non-hex content.
    pub fn from_hex(s: &str) -> Result<Self, crate::result::HygeError> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 64 {
            return Err(crate::result::HygeError::parse(
                "expected 64 hex characters (with optional 0x prefix)",
            ));
        }
        let mut bytes = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(chunk)
                .map_err(|_| crate::result::HygeError::parse("non-utf8 in hex input"))?;
            bytes[i] = u8::from_str_radix(hex, 16)
                .map_err(|_| crate::result::HygeError::parse("invalid hex digit"))?;
        }
        Ok(AssetId(bytes))
    }
}

impl From<blake3::Hash> for AssetId {
    fn from(hash: blake3::Hash) -> Self {
        AssetId(*hash.as_bytes())
    }
}

impl From<[u8; 32]> for AssetId {
    fn from(bytes: [u8; 32]) -> Self {
        AssetId(bytes)
    }
}

impl fmt::Display for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for AssetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AssetId({})", self.to_hex())
    }
}

/// Hex (de)serializer for `[u8; 32]`, inline so we don't pull in the `hex`
/// crate just for this. The on-the-wire format is a 64-character
/// lowercase hex string.
mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    /// Serializes the 32 bytes as a 64-char hex string.
    pub fn serialize<S: Serializer>(b: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        let mut hex = String::with_capacity(64);
        for byte in b {
            // `std::fmt::Write` is in the prelude in recent Rust editions,
            // but we import it explicitly to be portable to edition 2018.
            use std::fmt::Write;
            write!(hex, "{byte:02x}").expect("writing to String never fails");
        }
        s.serialize_str(&hex)
    }

    /// Deserializes a 64-char hex string back to 32 bytes.
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        let s = s.strip_prefix("0x").unwrap_or(&s);
        if s.len() != 64 {
            return Err(serde::de::Error::custom(
                "expected 64 hex characters (with optional 0x prefix)",
            ));
        }
        let mut out = [0u8; 32];
        for (i, chunk) in s.as_bytes().chunks(2).enumerate() {
            let hex = std::str::from_utf8(chunk).map_err(serde::de::Error::custom)?;
            out[i] = u8::from_str_radix(hex, 16).map_err(serde::de::Error::custom)?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_is_deterministic() {
        // R-010 acceptance: BLAKE3 hashing must be deterministic. Two
        // hashes of the same input must produce the same AssetId.
        let h1: AssetId = blake3::hash(b"hello world").into();
        let h2: AssetId = blake3::hash(b"hello world").into();
        assert_eq!(h1, h2);
        assert_eq!(h1.as_bytes(), h2.as_bytes());
    }

    #[test]
    fn different_inputs_produce_different_ids() {
        let a: AssetId = blake3::hash(b"alpha").into();
        let b: AssetId = blake3::hash(b"beta").into();
        assert_ne!(a, b);
    }

    #[test]
    fn hex_round_trip() {
        let original: AssetId = blake3::hash(b"sample data").into();
        let hex = original.to_hex();
        assert_eq!(hex.len(), 64);
        let round_tripped = AssetId::from_hex(&hex).expect("hex parse");
        assert_eq!(original, round_tripped);
    }

    #[test]
    fn hex_accepts_0x_prefix() {
        let original: AssetId = blake3::hash(b"x").into();
        let hex = format!("0x{}", original.to_hex());
        let parsed = AssetId::from_hex(&hex).expect("hex parse with prefix");
        assert_eq!(original, parsed);
    }

    #[test]
    fn hex_rejects_wrong_length() {
        let err = AssetId::from_hex("abc").unwrap_err();
        assert!(matches!(err, crate::result::HygeError::Parse(_)));
    }

    #[test]
    fn hex_rejects_non_hex() {
        let bad = "z".repeat(64);
        let err = AssetId::from_hex(&bad).unwrap_err();
        assert!(matches!(err, crate::result::HygeError::Parse(_)));
    }

    #[test]
    fn from_blake3_hash_and_from_bytes_match() {
        let hash = blake3::hash(b"x");
        let from_hash = AssetId::from(hash);
        let from_bytes = AssetId::from(*hash.as_bytes());
        assert_eq!(from_hash, from_bytes);
    }

    #[test]
    fn null_sentinel() {
        assert!(AssetId::NULL.is_null());
        let non_null: AssetId = blake3::hash(b"x").into();
        assert!(!non_null.is_null());
    }

    #[test]
    fn display_matches_hex() {
        let id: AssetId = blake3::hash(b"display").into();
        assert_eq!(format!("{id}"), id.to_hex());
    }
}
