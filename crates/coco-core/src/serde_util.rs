//! Small serde helpers for shapes serde's own derive/std impls don't cover
//! directly — currently just fixed-size byte arrays wider than serde's
//! built-in array impl ceiling (`[T; 1..=32]`; see `ssc.rs`'s
//! `ram: [u8; 512]`).

use serde::{Deserializer, Serializer};

/// `#[serde(with = "crate::serde_util::byte_array")]` for a `[u8; N]` field
/// with `N` outside serde's built-in array range. Serializes as CBOR-native
/// bytes ([`serde_bytes`]'s wire format, same as `SystemBus::ram`) and
/// deserializes with an exact length check — a mismatch means the snapshot
/// came from a build with a different `N`, and must be rejected rather than
/// silently truncated or zero-padded.
pub mod byte_array {
    use serde::de::Error as _;

    use super::{Deserializer, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serde_bytes::serialize(bytes.as_slice(), serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let bytes: Vec<u8> = serde_bytes::deserialize(deserializer)?;
        let len = bytes.len();
        <[u8; N]>::try_from(bytes)
            .map_err(|_| D::Error::custom(format!("expected {N} bytes, got {len}")))
    }
}
