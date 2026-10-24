// SPDX-License-Identifier: GPL-2.0-or-later
//! Versioned, self-describing save-state envelope.
//!
//! Frontends store states opaquely, but a state must never be restored into a
//! different core, a different state-format revision or different content. The
//! envelope carries that identity and an integrity check so a corrupt or
//! foreign blob is rejected *before* it reaches a running machine.
//!
//! Cores should encode their backend payload with [`StateEnvelope::encode`] and
//! decode with [`StateEnvelope::decode_for`]. `content_hash` is
//! [`content_hash`] of the content bytes (a ROM), or `0` for cores whose state
//! is not tied to a single content file (for example an RGSS directory).

use std::error::Error;
use std::fmt;

/// Bytes at the start of every encoded state.
pub const STATE_MAGIC: &[u8; 8] = b"RSXPSAV\0";
/// Current envelope layout version. Bump only for an incompatible layout.
pub const STATE_ENVELOPE_VERSION: u16 = 1;
/// Upper bound on a single state payload. Generous enough for DS memory
/// snapshots while keeping a corrupt length field from requesting terabytes.
pub const MAX_STATE_PAYLOAD: usize = 256 * 1024 * 1024;

const HEADER_BYTES: usize = 32;
const CHECKSUM_BYTES: usize = 8;

/// Integrity/identity failure while encoding or decoding a state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateError {
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    LengthMismatch,
    TooLarge,
    Checksum,
    WrongCore { expected: String, actual: String },
    WrongRevision { expected: u32, actual: u32 },
    WrongContent { expected: u64, actual: u64 },
    EmptyPayload,
    CoreIdTooLong,
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => f.write_str("state does not start with the RSXP magic"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported state envelope version {version}")
            }
            Self::Truncated => f.write_str("state is truncated"),
            Self::LengthMismatch => f.write_str("state length does not match its header"),
            Self::TooLarge => f.write_str("state payload exceeds the contract limit"),
            Self::Checksum => f.write_str("state checksum does not match its payload"),
            Self::WrongCore { expected, actual } => {
                write!(f, "state belongs to core {actual}, not {expected}")
            }
            Self::WrongRevision { expected, actual } => write!(
                f,
                "state format revision {actual} is incompatible with revision {expected}"
            ),
            Self::WrongContent { expected, actual } => write!(
                f,
                "state belongs to content {actual:016x}, not {expected:016x}"
            ),
            Self::EmptyPayload => f.write_str("state payload is empty"),
            Self::CoreIdTooLong => f.write_str("core id is too long for the state envelope"),
        }
    }
}

impl Error for StateError {}

/// FNV-1a over arbitrary bytes. Stable across releases and platforms; used for
/// content identity, not for security.
pub fn content_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A decoded save state with the identity of the machine it belongs to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateEnvelope {
    /// Stable [`crate::CoreInfo::id`] of the producing core.
    pub core_id: String,
    /// Producer's [`crate::CoreInfo::state_revision`].
    pub revision: u32,
    /// [`content_hash`] of the content, or `0` when not content-specific.
    pub content_hash: u64,
    /// Opaque backend bytes.
    pub payload: Vec<u8>,
}

impl StateEnvelope {
    pub fn new(
        core_id: impl Into<String>,
        revision: u32,
        content_hash: u64,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            core_id: core_id.into(),
            revision,
            content_hash,
            payload,
        }
    }

    /// Serializes the envelope into a single self-validating blob.
    pub fn encode(&self) -> Result<Vec<u8>, StateError> {
        if self.payload.is_empty() {
            return Err(StateError::EmptyPayload);
        }
        if self.payload.len() > MAX_STATE_PAYLOAD {
            return Err(StateError::TooLarge);
        }
        let id = self.core_id.as_bytes();
        let id_len = u16::try_from(id.len()).map_err(|_| StateError::CoreIdTooLong)?;
        let total = HEADER_BYTES
            .checked_add(id.len())
            .and_then(|size| size.checked_add(self.payload.len()))
            .and_then(|size| size.checked_add(CHECKSUM_BYTES))
            .ok_or(StateError::TooLarge)?;
        let mut out = Vec::with_capacity(total);
        out.extend_from_slice(STATE_MAGIC);
        out.extend_from_slice(&STATE_ENVELOPE_VERSION.to_le_bytes());
        out.extend_from_slice(&id_len.to_le_bytes());
        out.extend_from_slice(&self.revision.to_le_bytes());
        out.extend_from_slice(&self.content_hash.to_le_bytes());
        out.extend_from_slice(&(self.payload.len() as u64).to_le_bytes());
        out.extend_from_slice(id);
        out.extend_from_slice(&self.payload);
        let checksum = content_hash(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        Ok(out)
    }

    /// Parses and integrity-checks a blob without accepting it for a machine.
    pub fn decode(bytes: &[u8]) -> Result<Self, StateError> {
        if bytes.len() < HEADER_BYTES + CHECKSUM_BYTES {
            return Err(StateError::Truncated);
        }
        if &bytes[..8] != STATE_MAGIC {
            return Err(StateError::BadMagic);
        }
        let version = u16::from_le_bytes(bytes[8..10].try_into().unwrap());
        if version != STATE_ENVELOPE_VERSION {
            return Err(StateError::UnsupportedVersion(version));
        }
        let id_len = u16::from_le_bytes(bytes[10..12].try_into().unwrap()) as usize;
        let revision = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        let content_hash_value = u64::from_le_bytes(bytes[16..24].try_into().unwrap());
        let payload_len = u64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let payload_len = usize::try_from(payload_len).map_err(|_| StateError::TooLarge)?;
        if payload_len == 0 {
            return Err(StateError::EmptyPayload);
        }
        if payload_len > MAX_STATE_PAYLOAD {
            return Err(StateError::TooLarge);
        }
        let expected = HEADER_BYTES
            .checked_add(id_len)
            .and_then(|size| size.checked_add(payload_len))
            .and_then(|size| size.checked_add(CHECKSUM_BYTES))
            .ok_or(StateError::TooLarge)?;
        if expected != bytes.len() {
            return Err(StateError::LengthMismatch);
        }
        let checksum_at = bytes.len() - CHECKSUM_BYTES;
        let stored = u64::from_le_bytes(bytes[checksum_at..].try_into().unwrap());
        if stored != content_hash(&bytes[..checksum_at]) {
            return Err(StateError::Checksum);
        }
        let id_start = HEADER_BYTES;
        let id_end = id_start + id_len;
        let core_id = std::str::from_utf8(&bytes[id_start..id_end])
            .map_err(|_| StateError::BadMagic)?
            .to_owned();
        let payload_start = id_end;
        let payload_end = payload_start + payload_len;
        Ok(Self {
            core_id,
            revision,
            content_hash: content_hash_value,
            payload: bytes[payload_start..payload_end].to_vec(),
        })
    }

    /// Decodes a blob and verifies that it belongs to the expected machine.
    pub fn decode_for(
        bytes: &[u8],
        core_id: &str,
        revision: u32,
        content_hash: u64,
    ) -> Result<Self, StateError> {
        let state = Self::decode(bytes)?;
        if state.core_id != core_id {
            return Err(StateError::WrongCore {
                expected: core_id.to_owned(),
                actual: state.core_id,
            });
        }
        if state.revision != revision {
            return Err(StateError::WrongRevision {
                expected: revision,
                actual: state.revision,
            });
        }
        if state.content_hash != content_hash {
            return Err(StateError::WrongContent {
                expected: content_hash,
                actual: state.content_hash,
            });
        }
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> StateEnvelope {
        StateEnvelope::new("gba", 2, 0x1234, vec![1, 2, 3, 4])
    }

    #[test]
    fn round_trips_and_verifies_identity() {
        let encoded = state().encode().unwrap();
        let decoded = StateEnvelope::decode(&encoded).unwrap();
        assert_eq!(decoded, state());
        assert!(StateEnvelope::decode_for(&encoded, "gba", 2, 0x1234).is_ok());
        assert!(matches!(
            StateEnvelope::decode_for(&encoded, "nds", 2, 0x1234),
            Err(StateError::WrongCore { .. })
        ));
        assert!(matches!(
            StateEnvelope::decode_for(&encoded, "gba", 1, 0x1234),
            Err(StateError::WrongRevision { .. })
        ));
        assert!(matches!(
            StateEnvelope::decode_for(&encoded, "gba", 2, 0x1235),
            Err(StateError::WrongContent { .. })
        ));
    }

    #[test]
    fn rejects_corruption_truncation_and_empty_payloads() {
        let mut encoded = state().encode().unwrap();
        encoded[HEADER_BYTES + 2] ^= 0x40;
        assert_eq!(StateEnvelope::decode(&encoded), Err(StateError::Checksum));
        assert!(matches!(
            StateEnvelope::decode(&state().encode().unwrap()[..20]),
            Err(StateError::Truncated)
        ));
        assert_eq!(
            StateEnvelope::decode(b"not-a-rsxp-state").unwrap_err(),
            StateError::Truncated
        );
        assert_eq!(
            StateEnvelope::new("gba", 1, 0, Vec::new()).encode(),
            Err(StateError::EmptyPayload)
        );
    }

    #[test]
    fn trailing_bytes_are_rejected() {
        let mut encoded = state().encode().unwrap();
        encoded.push(0);
        assert_eq!(
            StateEnvelope::decode(&encoded),
            Err(StateError::LengthMismatch)
        );
    }
}
