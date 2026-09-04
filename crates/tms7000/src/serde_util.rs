//! `#[serde(with = "serde_util::byte_array")]` for the 128-byte register
//! file, which is wider than serde's built-in array impls (`[T; 0..=32]`).
//! Serializes as a sequence of bytes and rejects a length mismatch.

pub mod byte_array {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer, const N: usize>(
        bytes: &[u8; N],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        bytes.as_slice().serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> Result<[u8; N], D::Error> {
        let bytes: Vec<u8> = Vec::deserialize(deserializer)?;
        let len = bytes.len();
        <[u8; N]>::try_from(bytes)
            .map_err(|_| D::Error::custom(format!("expected {N} bytes, got {len}")))
    }
}
