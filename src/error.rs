//! Typed, exhaustive error taxonomy for every operation in the crate.
//!
//! The enum is deliberately **exhaustive** (no `#[non_exhaustive]`): callers
//! are expected to match every variant, and a new variant is a
//! semver-minor event by design — the same policy as `wire-kit`'s
//! `WireError`. No codec entry point panics, wraps, or reads out of bounds:
//! every malformed input resolves to one of these variants.

use core::fmt;

/// The single crate-level error for the frame codec and bit-timing calculator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanError {
    /// An identifier does not fit its width: 11-bit (standard) identifiers
    /// top out at `0x7FF`, 29-bit (extended) at `0x1FFF_FFFF`. Also surfaces
    /// for raw `SocketCAN` id words that carry bits outside the modeled space
    /// (the `CAN_ERR_FLAG` error-frame class is not modeled; `got` is the
    /// full id word, which then necessarily exceeds `max`).
    InvalidIdLength {
        /// Largest value the identifier width permits (`0x7FF` or
        /// `0x1FFF_FFFF`).
        max: u32,
        /// The offending value.
        got: u32,
    },
    /// A payload does not fit the frame's constraints: more than 8 bytes on
    /// a classic frame, more than 64 bytes anywhere, or a CAN FD length that
    /// is not one of the DLC-representable values (9–11, 13–15, … bytes are
    /// not encodable). `max` is the largest length that would have fit.
    DataTooLong {
        /// Largest permitted payload length for this frame type.
        max: usize,
        /// The offending length.
        got: usize,
    },
    /// A DLC value outside `0..=15`, or a classic-frame length byte outside
    /// `0..=8`.
    ///
    /// Note: classic *remote* frames may legitimately carry a refined DLC
    /// of 9–15 (the SocketCAN `len8_dlc` extension); this variant is for
    /// values outside the representable space.
    InvalidDlc(u8),
    /// A CAN FD-only flag (BRS or ESI) was set on a classic frame.
    FdBrsOnClassic,
    /// A remote frame that carries data bytes, or a CAN FD frame marked as
    /// remote (CAN FD abolished remote frames — both violations surface
    /// here).
    RemoteFrameWithData,
    /// The buffer is shorter than the `SocketCAN` frame format requires
    /// (16 bytes for `struct can_frame`, 72 for `struct canfd_frame`).
    BufferTooShort {
        /// Number of bytes the frame format requires.
        need: usize,
        /// Number of bytes actually available.
        have: usize,
    },
    /// A bit-timing calculation was given impossible parameters (zero
    /// baudrate, zero clock, a sample point outside `(0, 1)`) or no
    /// configuration under the Bosch segment limits achieves the request.
    InvalidTiming(&'static str),
}

impl fmt::Display for CanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::InvalidIdLength { max, got } => {
                write!(f, "identifier 0x{got:X} exceeds its width limit 0x{max:X}")
            }
            Self::DataTooLong { max, got } => {
                write!(
                    f,
                    "payload of {got} bytes exceeds the {max}-byte maximum for this frame type"
                )
            }
            Self::InvalidDlc(dlc) => {
                write!(
                    f,
                    "invalid DLC {dlc}: expected 0..=15 (0..=8 for classic frames)"
                )
            }
            Self::FdBrsOnClassic => {
                write!(f, "FD-only flag (BRS/ESI) set on a classic CAN frame")
            }
            Self::RemoteFrameWithData => {
                write!(
                    f,
                    "remote frame carrying data bytes, or FD frame marked remote"
                )
            }
            Self::BufferTooShort { need, have } => {
                write!(f, "buffer of {have} bytes is too short: need {need}")
            }
            Self::InvalidTiming(reason) => {
                write!(f, "invalid bit-timing parameters: {reason}")
            }
        }
    }
}

impl core::error::Error for CanError {}
