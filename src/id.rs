//! CAN identifiers — 11-bit standard and 29-bit extended, validated by
//! construction.
//!
//! The payloads are stored in the smallest machine word that fits the
//! declared width (`u11` → `u16`, `u29` → `u32`). The enum variants are
//! public tuples, so the type system alone cannot stop a caller from writing
//! `CanId::Standard(0xFFFF)`; the crate's contract is therefore
//! *defense in depth*: every validating constructor rejects out-of-range
//! values, and every crate entry point (`parse`, `to_bytes`, the arbitration
//! model, the DBC-side producers) re-validates before trusting a raw id.

use crate::error::CanError;

/// Largest 11-bit standard identifier.
pub const STANDARD_ID_MAX: u32 = 0x7FF;
/// Largest 29-bit extended identifier.
pub const EXTENDED_ID_MAX: u32 = 0x1FFF_FFFF;

// SocketCAN id-word flag bits (linux/can.h).
pub(crate) const CAN_EFF_FLAG: u32 = 0x8000_0000;
pub(crate) const CAN_RTR_FLAG: u32 = 0x4000_0000;
pub(crate) const CAN_ERR_FLAG: u32 = 0x2000_0000;
pub(crate) const CAN_EFF_MASK: u32 = 0x1FFF_FFFF;
pub(crate) const CAN_SFF_MASK: u32 = 0x7FF;

/// A CAN identifier: 11-bit (standard) or 29-bit (extended).
///
/// Use [`CanId::standard`] / [`CanId::extended`] (range-validated) rather
/// than constructing the tuple variants by hand.
///
/// ```
/// use can_core::CanId;
///
/// let std_id = CanId::standard(0x123).unwrap();
/// let ext_id = CanId::extended(0x01AB_CDEF).unwrap();
///
/// assert!(std_id.is_standard());
/// assert_eq!(ext_id.raw(), 0x01AB_CDEF);
///
/// // Out-of-range values are rejected, never truncated:
/// assert!(CanId::standard(0x800).is_err());
/// assert!(CanId::extended(0x2000_0000).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CanId {
    /// 11-bit standard identifier, `0x000..=0x7FF` (a `u11` in a `u16`).
    Standard(u16),
    /// 29-bit extended identifier, `0x000..=0x1FFF_FFFF` (a `u29` in a
    /// `u32`).
    Extended(u32),
}

impl CanId {
    /// Validates and wraps an 11-bit standard identifier.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidIdLength`] when `raw > 0x7FF`.
    pub fn standard(raw: u16) -> Result<Self, CanError> {
        if u32::from(raw) > STANDARD_ID_MAX {
            return Err(CanError::InvalidIdLength {
                max: STANDARD_ID_MAX,
                got: u32::from(raw),
            });
        }
        Ok(Self::Standard(raw))
    }

    /// Validates and wraps a 29-bit extended identifier.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidIdLength`] when `raw > 0x1FFF_FFFF`.
    pub fn extended(raw: u32) -> Result<Self, CanError> {
        if raw > EXTENDED_ID_MAX {
            return Err(CanError::InvalidIdLength {
                max: EXTENDED_ID_MAX,
                got: raw,
            });
        }
        Ok(Self::Extended(raw))
    }

    /// Interprets a raw `SocketCAN` id word (flag bits plus identifier, as
    /// carried in `struct can_frame::can_id`): bit 31 selects the extended
    /// (29-bit) format, bits 0–28 carry the identifier. Flag bits other
    /// than EFF are stripped; the `CAN_ERR_FLAG` error class is rejected —
    /// error frames are not modeled.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidIdLength`] for error-flagged words and for
    /// unflagged words whose identifier bits exceed the standard 11-bit
    /// width.
    pub fn from_raw_socketcan(word: u32) -> Result<Self, CanError> {
        if word & CAN_ERR_FLAG != 0 {
            return Err(CanError::InvalidIdLength {
                max: CAN_EFF_MASK,
                got: word,
            });
        }
        if word & CAN_EFF_FLAG != 0 {
            Self::extended(word & CAN_EFF_MASK)
        } else {
            let id = word & CAN_EFF_MASK;
            if id > CAN_SFF_MASK {
                return Err(CanError::InvalidIdLength {
                    max: CAN_SFF_MASK,
                    got: id,
                });
            }
            // id ≤ 0x7FF by the check above — no truncation.
            #[allow(clippy::cast_possible_truncation)]
            Ok(Self::Standard(id as u16))
        }
    }

    /// `true` for the 11-bit standard format.
    #[must_use]
    pub const fn is_standard(&self) -> bool {
        matches!(self, Self::Standard(_))
    }

    /// `true` for the 29-bit extended format.
    #[must_use]
    pub const fn is_extended(&self) -> bool {
        matches!(self, Self::Extended(_))
    }

    /// Identifier width in bits: 11 (standard) or 29 (extended).
    #[must_use]
    pub const fn width(&self) -> u8 {
        match self {
            Self::Standard(_) => 11,
            Self::Extended(_) => 29,
        }
    }

    /// The identifier value, widened to `u32`.
    #[must_use]
    pub const fn raw(&self) -> u32 {
        match *self {
            // u16 → u32 widening is lossless.
            Self::Standard(v) => v as u32,
            Self::Extended(v) => v,
        }
    }

    /// The `SocketCAN` id word for this identifier: the id in bits 0–28 plus
    /// the EFF flag (bit 31) for the extended format. RTR/ERR flags are
    /// added by the frame layer.
    #[must_use]
    pub const fn to_raw_socketcan(&self) -> u32 {
        match *self {
            Self::Standard(v) => v as u32,
            Self::Extended(v) => v | CAN_EFF_FLAG,
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests exercise edge values and index fixed-size buffers directly:
    // unwrap/expect/indexing are lifted at test scope by design.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn standard_boundary() {
        assert!(CanId::standard(0).is_ok());
        assert!(CanId::standard(0x7FF).is_ok());
        assert_eq!(
            CanId::standard(0x800),
            Err(CanError::InvalidIdLength {
                max: STANDARD_ID_MAX,
                got: 0x800
            })
        );
    }

    #[test]
    fn extended_boundary() {
        assert!(CanId::extended(0).is_ok());
        assert!(CanId::extended(EXTENDED_ID_MAX).is_ok());
        assert_eq!(
            CanId::extended(0x2000_0000),
            Err(CanError::InvalidIdLength {
                max: EXTENDED_ID_MAX,
                got: 0x2000_0000
            })
        );
    }

    #[test]
    fn raw_socketcan_roundtrip() {
        assert_eq!(CanId::from_raw_socketcan(0x123), Ok(CanId::Standard(0x123)));
        assert_eq!(
            CanId::from_raw_socketcan(0x123 | CAN_EFF_FLAG),
            Ok(CanId::Extended(0x123))
        );
        assert_eq!(
            CanId::Extended(0x1AB).to_raw_socketcan(),
            0x1AB | CAN_EFF_FLAG
        );
        assert_eq!(CanId::Standard(0x1AB).to_raw_socketcan(), 0x1AB);
        // 11-bit overflow without the EFF flag:
        assert!(CanId::from_raw_socketcan(0x800).is_err());
        // Error-frame class is not modeled:
        assert!(CanId::from_raw_socketcan(CAN_ERR_FLAG | 0x123).is_err());
        // Bits between SFF and EFF masks without the EFF flag:
        assert!(CanId::from_raw_socketcan(0x0000_1000).is_err());
    }

    #[test]
    fn widths() {
        assert_eq!(CanId::Standard(0).width(), 11);
        assert_eq!(CanId::Extended(0).width(), 29);
    }
}
