//! CAN 2.0 and CAN FD frame codec — identifier parsing, DLC handling,
//! bit-timing calculation, arbitration model.
//!
//! `can-core` is the estate's **L0 leaf** for Controller Area Network work:
//! a total, allocation-free-at-parse frame codec over the `SocketCAN` wire
//! format, plus the bit-timing and arbitration models every higher CAN
//! layer shares.
//!
//! # Zero-copy, total parsing
//!
//! [`CanFrame::parse`] borrows the payload straight out of the input buffer —
//! one buffer in, a typed view out, nothing copied. Every buffer access is
//! bounds-checked: malformed, truncated, or hostile input yields a typed
//! [`CanError`], never a panic (enforced by deny-level lints on the lib
//! target, a cargo-fuzz target, and miri).
//!
//! # The frame model
//!
//! A [`CanFrame`] is an identifier ([`CanId`]: 11-bit standard or 29-bit
//! extended), a payload of 0–64 bytes, the Data Length Code, and the flag
//! bits (`is_remote`, `is_fd`, `brs`, `error_state_indicator`). Classic CAN
//! payloads cap at 8 bytes with DLC = length; CAN FD maps DLC 0–15 through
//! the table `0,1,2,3,4,5,6,7,8,12,16,20,24,32,48,64`
//! ([`dlc_to_len`]/[`len_to_dlc`]). Remote frames carry no payload, and CAN
//! FD — which abolished remote frames — rejects them outright.
//!
//! # Bit timing
//!
//! [`timing::BitTiming::calculate`] solves the standard Bosch bit-timing
//! algorithm from baudrate and clock: prescaler search plus segment split,
//! sample point closest to the `CiA` default (87.5 % below 800 kbit/s, 75 %
//! above), rendered as SJA1000-style BTR register bytes via
//! [`timing::BitTiming::btr`].
//!
//! # Arbitration
//!
//! Lower identifiers win — with the wire-level subtlety that a standard
//! frame beats an extended frame sharing its first 11 id bits (dominant IDE
//! vs recessive SRR). [`arbitration::dominates`] models the full comparison;
//! CAN FD changes nothing at arbitration phase (BRS applies only after the
//! winner is decided).
//!
//! # Example
//!
//! ```
//! use can_core::{CanFrame, CanId, CanError};
//!
//! // Build and round-trip a CAN FD frame:
//! let id = CanId::extended(0x18FF_0102)?;
//! let frame = CanFrame::fd(id, &[0xDE, 0xAD, 0xBE, 0xEF], true, false)?;
//! let bytes = frame.to_bytes();
//! assert_eq!(bytes.len(), 72);
//! assert_eq!(CanFrame::parse(&bytes)?, frame);
//!
//! // Out-of-range ids are rejected, never truncated:
//! assert!(matches!(
//!     CanId::standard(0x800),
//!     Err(CanError::InvalidIdLength { max: 0x7FF, got: 0x800 })
//! ));
//! # Ok::<(), CanError>(())
//! ```
//!
//! # `no_std`
//!
//! The crate is `no_std` + `alloc` — it builds for embedded targets
//! (`thumbv7em-none-eabihf` is exercised in CI) and depends only on
//! `thiserror` for the error taxonomy.

#![no_std]
extern crate alloc;

mod error;
mod frame;
mod id;

pub mod arbitration;
pub mod timing;

pub use error::CanError;
pub use frame::{
    dlc_to_len, len_to_dlc, CanFrame, CLASSIC_FRAME_SIZE, CLASSIC_MAX_LEN, FD_DLC_TO_LEN,
    FD_FRAME_SIZE, FD_MAX_LEN,
};
pub use id::{CanId, EXTENDED_ID_MAX, STANDARD_ID_MAX};
