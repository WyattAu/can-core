//! CAN frames — classic CAN 2.0 and CAN FD, in `SocketCAN` wire format.
//!
//! [`CanFrame`] is a **zero-copy view**: `parse` borrows the payload region
//! straight out of the input buffer (no copy, no allocation), and `to_bytes`
//! writes the `SocketCAN` `struct can_frame` (16 bytes) or `struct canfd_frame`
//! (72 bytes) layout back into a fresh `Vec<u8>`.
//!
//! # DLC ↔ payload-length mapping
//!
//! Classic CAN (DLC 0–8): the DLC *is* the payload length in bytes.
//! CAN FD (DLC 0–15): the DLC is an index into the mapping table
//! `0,1,2,3,4,5,6,7,8,12,16,20,24,32,48,64` — DLC 9–15 encode payloads of
//! 12–64 bytes. Lengths that are not table entries (9–11, 13–15, … bytes)
//! are not encodable and rejected with [`CanError::DataTooLong`].
//!
//! # Totality
//!
//! `parse` never panics: every buffer access is bounds-checked and every
//! malformed input resolves to a typed [`CanError`].

use crate::error::CanError;
use crate::id::{CanId, CAN_EFF_MASK, CAN_ERR_FLAG, CAN_RTR_FLAG};
use alloc::vec::Vec;

/// Largest classic CAN 2.0 payload.
pub const CLASSIC_MAX_LEN: usize = 8;
/// Largest CAN FD payload.
pub const FD_MAX_LEN: usize = 64;
/// Size of `struct can_frame` — the `SocketCAN` classic wire format.
pub const CLASSIC_FRAME_SIZE: usize = 16;
/// Size of `struct canfd_frame` — the `SocketCAN` CAN FD wire format.
pub const FD_FRAME_SIZE: usize = 72;

/// DLC (0–15) → CAN FD payload length in bytes. The first nine entries also
/// serve classic CAN, where the DLC equals the length directly.
pub const FD_DLC_TO_LEN: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64];

/// Maps a DLC (0–15) to its CAN FD payload length.
///
/// # Errors
///
/// [`CanError::InvalidDlc`] when `dlc > 15`.
pub fn dlc_to_len(dlc: u8) -> Result<usize, CanError> {
    FD_DLC_TO_LEN
        .get(usize::from(dlc))
        .copied()
        .ok_or(CanError::InvalidDlc(dlc))
}

/// Maps a CAN FD payload length to its DLC.
///
/// # Errors
///
/// [`CanError::DataTooLong`] when `len` exceeds 64 bytes or is not a
/// DLC-representable length (9–11, 13–15, 17–19, …); `max` then reports the
/// largest length that *would* fit.
pub fn len_to_dlc(len: usize) -> Result<u8, CanError> {
    // Linear scan of a 16-entry table: the first hit is the unique entry.
    for (dlc, table_len) in FD_DLC_TO_LEN.iter().enumerate() {
        if *table_len == len {
            // dlc < FD_DLC_TO_LEN.len() == 16 by construction — no truncation.
            #[allow(clippy::cast_possible_truncation)]
            return Ok(dlc as u8);
        }
    }
    // Not a table entry: report the largest length that would fit.
    let max = FD_DLC_TO_LEN
        .iter()
        .rev()
        .find(|candidate| **candidate <= len)
        .copied()
        .unwrap_or(0);
    Err(CanError::DataTooLong { max, got: len })
}

/// A CAN 2.0 or CAN FD frame — a borrowed, zero-copy view over a payload.
///
/// Construct with [`CanFrame::new`] (classic data), [`CanFrame::fd`] (CAN FD
/// data), [`CanFrame::remote`] (classic RTR), or the fully explicit
/// [`CanFrame::from_parts`]; parse from `SocketCAN` bytes with
/// [`CanFrame::parse`]; serialize with [`CanFrame::to_bytes`].
///
/// ```
/// use can_core::{CanFrame, CanId};
///
/// let id = CanId::standard(0x123).unwrap();
/// let payload = [0xDE, 0xAD, 0xBE, 0xEF];
/// let frame = CanFrame::new(id, &payload).unwrap();
///
/// let bytes = frame.to_bytes();
/// assert_eq!(bytes.len(), 16); // struct can_frame
/// assert_eq!(&bytes[8..12], &payload);
///
/// let reparsed = CanFrame::parse(&bytes).unwrap();
/// assert_eq!(frame, reparsed);
/// ```
// The CAN frame format carries exactly these flag bits (RTR, FD, BRS,
// ESI) — the bool count is the domain, not a design smell.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanFrame<'a> {
    id: CanId,
    data: &'a [u8],
    dlc: u8,
    is_remote: bool,
    is_fd: bool,
    brs: bool,
    error_state_indicator: bool,
}

impl<'a> CanFrame<'a> {
    /// Builds a classic CAN 2.0 data frame; the DLC is the payload length.
    ///
    /// # Errors
    ///
    /// [`CanError::DataTooLong`] for payloads over 8 bytes, and
    /// [`CanError::InvalidIdLength`] for an out-of-range id.
    pub fn new(id: CanId, data: &'a [u8]) -> Result<Self, CanError> {
        if data.len() > CLASSIC_MAX_LEN {
            return Err(CanError::DataTooLong {
                max: CLASSIC_MAX_LEN,
                got: data.len(),
            });
        }
        // len ≤ 8 by the check above — no truncation.
        #[allow(clippy::cast_possible_truncation)]
        Self::from_parts(id, data, data.len() as u8, false, false, false, false)
    }

    /// Builds a CAN FD data frame.
    ///
    /// # Errors
    ///
    /// [`CanError::DataTooLong`] for payloads that are not
    /// DLC-representable (or exceed 64 bytes), and
    /// [`CanError::InvalidIdLength`] for an out-of-range id.
    pub fn fd(id: CanId, data: &'a [u8], brs: bool, esi: bool) -> Result<Self, CanError> {
        let dlc = len_to_dlc(data.len())?;
        Self::from_parts(id, data, dlc, false, true, brs, esi)
    }

    /// Builds a classic remote-transmission-request frame: no payload, a
    /// DLC of `dlc` (the requested length). DLC 9–15 models the legacy
    /// SocketCAN refinement (`len8_dlc`) for remote frames requesting more
    /// than 8 bytes.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidDlc`] when `dlc > 15`, [`CanError::InvalidIdLength`]
    /// for an out-of-range id.
    pub fn remote(id: CanId, dlc: u8) -> Result<Self, CanError> {
        Self::from_parts(id, &[], dlc, true, false, false, false)
    }

    /// Fully explicit constructor — every field is validated.
    ///
    /// # Errors
    ///
    /// All [`CanError`] variants are reachable, depending on the field
    /// combination: id range, payload/DLC consistency, FD-vs-classic flag
    /// constraints, and the remote-frame rules (remote frames carry no data;
    /// CAN FD frames cannot be remote).
    #[allow(clippy::fn_params_excessive_bools)] // the format's four flag bits
    pub fn from_parts(
        id: CanId,
        data: &'a [u8],
        dlc: u8,
        is_remote: bool,
        is_fd: bool,
        brs: bool,
        error_state_indicator: bool,
    ) -> Result<Self, CanError> {
        validate_id(id)?;
        if dlc > 15 {
            return Err(CanError::InvalidDlc(dlc));
        }
        if is_remote {
            if !data.is_empty() {
                return Err(CanError::RemoteFrameWithData);
            }
            if is_fd {
                // CAN FD abolished remote frames (ISO 11898-1).
                return Err(CanError::RemoteFrameWithData);
            }
            // Remote frames carry no payload. DLC 9–15 is the legacy
            // SocketCAN refinement (len8_dlc) — a classic remote frame
            // requesting more than 8 bytes.
        } else if is_fd {
            // BRS/ESI are legal on FD frames — no constraint beyond the
            // payload/DLC consistency check.
            let expected = len_to_dlc(data.len())?;
            if expected != dlc {
                return Err(CanError::InvalidDlc(dlc));
            }
        } else {
            if data.len() > CLASSIC_MAX_LEN {
                return Err(CanError::DataTooLong {
                    max: CLASSIC_MAX_LEN,
                    got: data.len(),
                });
            }
            if brs || error_state_indicator {
                return Err(CanError::FdBrsOnClassic);
            }
            // Classic: DLC == payload length, or the refined legacy form —
            // an 8-byte payload carrying DLC 9–15 (as kernels report for
            // adapters that transmit such frames).
            let refined = data.len() == CLASSIC_MAX_LEN && (9..=15).contains(&dlc);
            if usize::from(dlc) != data.len() && !refined {
                return Err(CanError::InvalidDlc(dlc));
            }
        }
        Ok(Self {
            id,
            data,
            dlc,
            is_remote,
            is_fd,
            brs,
            error_state_indicator,
        })
    }

    /// Parses one frame in `SocketCAN` wire format: exactly 16 bytes
    /// (`struct can_frame`, classic) or exactly 72 bytes (`struct
    /// canfd_frame`, CAN FD). The payload borrows from `buf`.
    ///
    /// # Errors
    ///
    /// [`CanError::BufferTooShort`] under 16 bytes,
    /// [`CanError::DataTooLong`] above 16 bytes when not exactly 72,
    /// [`CanError::InvalidIdLength`] for error-flagged or over-wide id
    /// words, [`CanError::InvalidDlc`] for out-of-range length bytes, and
    /// [`CanError::RemoteFrameWithData`] for an RTR flag inside an FD frame.
    pub fn parse(buf: &'a [u8]) -> Result<Self, CanError> {
        if buf.len() < CLASSIC_FRAME_SIZE {
            return Err(CanError::BufferTooShort {
                need: CLASSIC_FRAME_SIZE,
                have: buf.len(),
            });
        }
        let Some(word_bytes) = buf.get(0..4) else {
            return Err(CanError::BufferTooShort {
                need: 4,
                have: buf.len(),
            });
        };
        let mut word = [0u8; 4];
        word.copy_from_slice(word_bytes);
        let raw = u32::from_le_bytes(word);

        match buf.len() {
            CLASSIC_FRAME_SIZE => parse_classic(buf, raw),
            FD_FRAME_SIZE => parse_fd(buf, raw),
            _ => Err(CanError::DataTooLong {
                max: CLASSIC_FRAME_SIZE,
                got: buf.len(),
            }),
        }
    }

    /// Serializes to the `SocketCAN` wire format: 16 bytes for classic frames
    /// (with the RTR flag for remote frames), 72 bytes for CAN FD frames
    /// (BRS/ESI in the flags byte).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut word = self.id.to_raw_socketcan();
        if self.is_remote {
            word |= CAN_RTR_FLAG;
        }
        if self.is_fd {
            let mut out = Vec::with_capacity(FD_FRAME_SIZE);
            out.extend_from_slice(&word.to_le_bytes());
            // len (0..=64), flags (BRS = 0x01, ESI = 0x02), two reserved
            // bytes, then the payload. Invariant: len ≤ 64.
            #[allow(clippy::cast_possible_truncation)]
            out.push(self.data.len() as u8);
            out.push(u8::from(self.brs) | (u8::from(self.error_state_indicator) << 1));
            out.push(0);
            out.push(0);
            out.extend_from_slice(self.data);
            out.resize(FD_FRAME_SIZE, 0);
            out
        } else {
            let mut out = Vec::with_capacity(CLASSIC_FRAME_SIZE);
            out.extend_from_slice(&word.to_le_bytes());
            // Length byte: the requested length for RTR frames (capped at
            // 8 — a refined DLC 9–15 rides in len8_dlc), else the payload
            // length. Invariant: ≤ 8 either way.
            #[allow(clippy::cast_possible_truncation)]
            let len = if self.is_remote {
                self.dlc.min(CLASSIC_MAX_LEN as u8)
            } else {
                self.data.len() as u8
            };
            out.push(len);
            out.push(0); // __pad
            out.push(0); // __res0
                         // len8_dlc: the precise DLC, refining the length byte exactly
                         // when it lies in 9..=15 (classic 8-byte payloads, and legacy
                         // RTR frames requesting more than 8 bytes).
            out.push(self.dlc);
            if !self.is_remote {
                out.extend_from_slice(self.data);
            }
            out.resize(CLASSIC_FRAME_SIZE, 0);
            out
        }
    }

    /// The frame's identifier.
    #[must_use]
    pub const fn id(&self) -> CanId {
        self.id
    }

    /// The payload — empty for remote frames.
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The Data Length Code (0–8 classic, 0–15 CAN FD).
    #[must_use]
    pub const fn dlc(&self) -> u8 {
        self.dlc
    }

    /// `true` for remote-transmission-request frames.
    #[must_use]
    pub const fn is_remote(&self) -> bool {
        self.is_remote
    }

    /// `true` for CAN FD frames.
    #[must_use]
    pub const fn is_fd(&self) -> bool {
        self.is_fd
    }

    /// Bit-rate switch: the FD arbitration phase runs at the nominal rate,
    /// the data phase at a faster rate. Meaningful only when [`is_fd`].
    ///
    /// [`is_fd`]: Self::is_fd
    #[must_use]
    pub const fn brs(&self) -> bool {
        self.brs
    }

    /// Error state indicator (FD only): the transmitter's error-counter
    /// state, carried transparently for network error bookkeeping.
    #[must_use]
    pub const fn error_state_indicator(&self) -> bool {
        self.error_state_indicator
    }
}

/// Validates an id handed to a public constructor.
fn validate_id(id: CanId) -> Result<(), CanError> {
    match id {
        CanId::Standard(v) if u32::from(v) > crate::id::STANDARD_ID_MAX => {
            Err(CanError::InvalidIdLength {
                max: crate::id::STANDARD_ID_MAX,
                got: u32::from(v),
            })
        }
        CanId::Extended(v) if v > crate::id::EXTENDED_ID_MAX => Err(CanError::InvalidIdLength {
            max: crate::id::EXTENDED_ID_MAX,
            got: v,
        }),
        _ => Ok(()),
    }
}

/// Parses a 16-byte `struct can_frame`.
fn parse_classic(buf: &[u8], raw: u32) -> Result<CanFrame<'_>, CanError> {
    let id = CanId::from_raw_socketcan(raw)?;
    let is_remote = raw & CAN_RTR_FLAG != 0;

    let Some(&len_byte) = buf.get(4) else {
        return Err(CanError::BufferTooShort {
            need: 5,
            have: buf.len(),
        });
    };
    // The classic length byte is a true length (0..=8); kernels emit the
    // refined DLC separately in len8_dlc.
    if usize::from(len_byte) > CLASSIC_MAX_LEN {
        return Err(CanError::InvalidDlc(len_byte));
    }
    let len = usize::from(len_byte);

    // len8_dlc (byte 7): when the length byte is 8 and len8_dlc carries a
    // refined DLC 9–15 (kernels surface legacy "DLC > 8" classic frames —
    // data or remote — this way), it refines the DLC; otherwise it is
    // ignored. The payload stays at 8 bytes — classic CAN cannot carry
    // more.
    let dlc = match buf.get(7) {
        Some(&dlc8) if len == CLASSIC_MAX_LEN && (9..=15).contains(&dlc8) => dlc8,
        _ => len_byte,
    };

    let data = if is_remote {
        &[]
    } else {
        buf.get(8..8 + len).ok_or(CanError::BufferTooShort {
            need: 8 + len,
            have: buf.len(),
        })?
    };

    Ok(CanFrame {
        id,
        data,
        dlc,
        is_remote,
        is_fd: false,
        brs: false,
        error_state_indicator: false,
    })
}

/// Parses a 72-byte `struct canfd_frame`.
fn parse_fd(buf: &[u8], raw: u32) -> Result<CanFrame<'_>, CanError> {
    // CAN FD abolished remote frames and error frames: neither flag may
    // appear in an FD id word.
    if raw & CAN_RTR_FLAG != 0 {
        return Err(CanError::RemoteFrameWithData);
    }
    if raw & CAN_ERR_FLAG != 0 {
        return Err(CanError::InvalidIdLength {
            max: CAN_EFF_MASK,
            got: raw,
        });
    }
    let id = CanId::from_raw_socketcan(raw)?;

    let Some(&len_byte) = buf.get(4) else {
        return Err(CanError::BufferTooShort {
            need: 5,
            have: buf.len(),
        });
    };
    if usize::from(len_byte) > FD_MAX_LEN {
        return Err(CanError::DataTooLong {
            max: FD_MAX_LEN,
            got: usize::from(len_byte),
        });
    }
    let len = usize::from(len_byte);
    let Ok(dlc) = len_to_dlc(len) else {
        // Not a DLC-representable FD length (9–11, 13–15, …).
        return Err(CanError::DataTooLong {
            max: largest_fd_len_below(len),
            got: len,
        });
    };

    let flags = buf.get(5).copied().unwrap_or(0);
    let brs = flags & 0x01 != 0;
    let error_state_indicator = flags & 0x02 != 0;
    // Remaining flag bits are reserved by the kernel ABI; ignored.

    let data = buf.get(8..8 + len).ok_or(CanError::BufferTooShort {
        need: 8 + len,
        have: buf.len(),
    })?;

    Ok(CanFrame {
        id,
        data,
        dlc,
        is_remote: false,
        is_fd: true,
        brs,
        error_state_indicator,
    })
}

/// Largest DLC-representable FD length `<= len` (0 when none).
fn largest_fd_len_below(len: usize) -> usize {
    FD_DLC_TO_LEN
        .iter()
        .rev()
        .find(|candidate| **candidate <= len)
        .copied()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    // Tests exercise edge values and index fixed-size buffers directly:
    // unwrap/expect/indexing are lifted at test scope by design.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use crate::id::{CAN_EFF_FLAG, CAN_SFF_MASK};

    #[test]
    fn dlc_table_is_complete_and_monotone() {
        assert_eq!(FD_DLC_TO_LEN.len(), 16);
        assert_eq!(FD_DLC_TO_LEN[0], 0);
        assert_eq!(FD_DLC_TO_LEN[8], 8);
        assert_eq!(FD_DLC_TO_LEN[15], 64);
        for window in FD_DLC_TO_LEN.windows(2) {
            assert!(window[0] < window[1], "table must be strictly increasing");
        }
        for (dlc, len) in FD_DLC_TO_LEN.iter().enumerate() {
            // dlc < 16 by construction — no truncation.
            #[allow(clippy::cast_possible_truncation)]
            let dlc = dlc as u8;
            assert_eq!(dlc_to_len(dlc), Ok(*len));
            assert_eq!(len_to_dlc(*len), Ok(dlc));
        }
    }

    #[test]
    fn dlc_errors() {
        assert_eq!(dlc_to_len(16), Err(CanError::InvalidDlc(16)));
        assert_eq!(
            len_to_dlc(11),
            Err(CanError::DataTooLong { max: 8, got: 11 })
        );
        assert_eq!(
            len_to_dlc(13),
            Err(CanError::DataTooLong { max: 12, got: 13 })
        );
        assert_eq!(
            len_to_dlc(65),
            Err(CanError::DataTooLong { max: 64, got: 65 })
        );
    }

    #[test]
    fn classic_wire_layout() {
        let id = CanId::standard(0x123).unwrap();
        let frame = CanFrame::new(id, &[1, 2, 3]).unwrap();
        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[0..4], &0x123_u32.to_le_bytes());
        assert_eq!(bytes[4], 3);
        assert_eq!(&bytes[8..11], &[1, 2, 3]);
    }

    #[test]
    fn classic_len8_dlc_refinement() {
        // len == 8 with len8_dlc == 12: a refined DLC 12 on an 8-byte
        // classic payload (legacy "DLC > 8" frames from real adapters).
        let mut buf = [0u8; 16];
        buf[0..4].copy_from_slice(&0x456_u32.to_le_bytes());
        buf[4] = 8;
        buf[7] = 12;
        let frame = CanFrame::parse(&buf).unwrap();
        assert_eq!(frame.dlc(), 12);
        assert_eq!(frame.data().len(), 8);
        // The refined frame is constructible and round-trips byte-exactly.
        let payload = buf.get(8..16).unwrap();
        let rebuilt = CanFrame::from_parts(
            CanId::standard(0x456).unwrap(),
            payload,
            12,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        assert_eq!(rebuilt, frame);
        let out = frame.to_bytes();
        assert_eq!(out, buf);
        // Refined forms below DLC 9 or on shorter payloads are ignored:
        buf[7] = 5; // len8_dlc <= 8 carries no refinement
        assert_eq!(CanFrame::parse(&buf).unwrap().dlc(), 8);
        buf[4] = 3;
        buf[7] = 12;
        assert_eq!(CanFrame::parse(&buf).unwrap().dlc(), 3);
    }

    #[test]
    fn fd_wire_layout() {
        let id = CanId::extended(0x18FF_0102).unwrap();
        let payload = [7u8; 12]; // DLC 9
        let frame = CanFrame::fd(id, &payload, true, false).unwrap();
        assert_eq!(frame.dlc(), 9);
        let bytes = frame.to_bytes();
        assert_eq!(bytes.len(), 72);
        assert_eq!(
            &bytes[0..4],
            &(0x18FF_0102_u32 | CAN_EFF_FLAG).to_le_bytes()
        );
        assert_eq!(bytes[4], 12); // true length byte
        assert_eq!(bytes[5], 0x01); // BRS
        assert_eq!(&bytes[8..20], &payload);

        let reparsed = CanFrame::parse(&bytes).unwrap();
        assert!(reparsed.is_fd());
        assert!(reparsed.brs());
        assert!(!reparsed.error_state_indicator());
        assert_eq!(frame, reparsed);
    }

    #[test]
    fn fd_all_lengths_roundtrip() {
        let id = CanId::standard(0x7FF).unwrap();
        for dlc in 0_u8..=15 {
            let len = dlc_to_len(dlc).unwrap();
            let payload = [0xAB_u8; 64];
            let frame = CanFrame::fd(id, &payload[..len], false, false).unwrap();
            let bytes = frame.to_bytes();
            let reparsed = CanFrame::parse(&bytes).unwrap();
            assert_eq!(reparsed, frame);
            assert_eq!(reparsed.dlc(), dlc);
        }
    }

    #[test]
    fn parse_errors() {
        // Too short:
        assert_eq!(
            CanFrame::parse(&[0u8; 15]),
            Err(CanError::BufferTooShort { need: 16, have: 15 })
        );
        // Wrong total size:
        assert_eq!(
            CanFrame::parse(&[0u8; 17]),
            Err(CanError::DataTooLong { max: 16, got: 17 })
        );
        // Classic length byte > 8:
        let mut buf = [0u8; 16];
        buf[4] = 9;
        assert_eq!(CanFrame::parse(&buf), Err(CanError::InvalidDlc(9)));
        // Error-frame class:
        let mut err_buf = [0u8; 16];
        err_buf[0..4].copy_from_slice(&(CAN_ERR_FLAG | 0x123).to_le_bytes());
        assert!(matches!(
            CanFrame::parse(&err_buf),
            Err(CanError::InvalidIdLength { .. })
        ));
        // Standard-classic id with bits above 0x7FF:
        let mut wide = [0u8; 16];
        wide[0..4].copy_from_slice(&0x1234_u32.to_le_bytes());
        assert!(matches!(
            CanFrame::parse(&wide),
            Err(CanError::InvalidIdLength {
                max: CAN_SFF_MASK,
                ..
            })
        ));
        // RTR flag inside an FD-sized frame:
        let mut fd_rtr = [0u8; 72];
        fd_rtr[0..4].copy_from_slice(&(CAN_EFF_FLAG | CAN_RTR_FLAG | 0x123).to_le_bytes());
        assert_eq!(CanFrame::parse(&fd_rtr), Err(CanError::RemoteFrameWithData));
        // FD length byte not DLC-representable:
        let mut fd_bad = [0u8; 72];
        fd_bad[4] = 11;
        assert_eq!(
            CanFrame::parse(&fd_bad),
            Err(CanError::DataTooLong { max: 8, got: 11 })
        );
        // FD length byte > 64:
        let mut fd_huge = [0u8; 72];
        fd_huge[4] = 65;
        assert_eq!(
            CanFrame::parse(&fd_huge),
            Err(CanError::DataTooLong { max: 64, got: 65 })
        );
        // ESI flag survives the round trip:
        let mut fd_esi = [0u8; 72];
        fd_esi[4] = 8;
        fd_esi[5] = 0x02;
        let esi = CanFrame::parse(&fd_esi).unwrap();
        assert!(esi.error_state_indicator());
        assert!(!esi.brs());
    }

    #[test]
    fn constructor_errors() {
        let id = CanId::standard(0x123).unwrap();
        let nine = [0u8; 9];
        // Classic with 9 bytes:
        assert_eq!(
            CanFrame::new(id, &nine),
            Err(CanError::DataTooLong { max: 8, got: 9 })
        );
        // FD with a non-representable length:
        assert_eq!(
            CanFrame::fd(id, &nine, false, false),
            Err(CanError::DataTooLong { max: 8, got: 9 })
        );
        // BRS on a classic frame:
        assert_eq!(
            CanFrame::from_parts(id, &[], 0, false, false, true, false),
            Err(CanError::FdBrsOnClassic)
        );
        // ESI on a classic frame:
        assert_eq!(
            CanFrame::from_parts(id, &[], 0, false, false, false, true),
            Err(CanError::FdBrsOnClassic)
        );
        // Remote frame with data:
        assert_eq!(
            CanFrame::from_parts(id, &[1], 1, true, false, false, false),
            Err(CanError::RemoteFrameWithData)
        );
        // FD remote frame:
        assert_eq!(
            CanFrame::from_parts(id, &[], 0, true, true, false, false),
            Err(CanError::RemoteFrameWithData)
        );
        // Remote DLC > 15:
        assert_eq!(CanFrame::remote(id, 16), Err(CanError::InvalidDlc(16)));
        // Refined remote DLC (legacy len8_dlc): serializes as len 8 +
        // len8_dlc 12, and round-trips.
        let refined = CanFrame::remote(id, 12).unwrap();
        assert_eq!(refined.dlc(), 12);
        let bytes = refined.to_bytes();
        assert_eq!(bytes[4], 8);
        assert_eq!(bytes[7], 12);
        assert_eq!(CanFrame::parse(&bytes), Ok(refined));
        // DLC > 15:
        assert_eq!(
            CanFrame::from_parts(id, &[], 16, false, false, false, false),
            Err(CanError::InvalidDlc(16))
        );
        // Classic DLC inconsistent with the payload length:
        assert_eq!(
            CanFrame::from_parts(id, &[1, 2], 3, false, false, false, false),
            Err(CanError::InvalidDlc(3))
        );
        // FD DLC inconsistent with the payload length:
        assert_eq!(
            CanFrame::from_parts(id, &[1, 2], 1, false, true, false, false),
            Err(CanError::InvalidDlc(1))
        );
        // Out-of-range id via a hand-built enum value (defense in depth):
        assert_eq!(
            CanFrame::from_parts(CanId::Standard(0xFFFF), &[], 0, false, false, false, false),
            Err(CanError::InvalidIdLength {
                max: 0x7FF,
                got: 0xFFFF
            })
        );
    }

    #[test]
    fn remote_frame_wire_roundtrip() {
        let id = CanId::extended(0x0123_4567).unwrap();
        let frame = CanFrame::remote(id, 3).unwrap();
        assert!(frame.is_remote());
        assert_eq!(frame.data(), &[]);
        let bytes = frame.to_bytes();
        assert_eq!(
            &bytes[0..4],
            &(0x0123_4567_u32 | CAN_EFF_FLAG | CAN_RTR_FLAG).to_le_bytes()
        );
        assert_eq!(bytes[4], 3);
        let reparsed = CanFrame::parse(&bytes).unwrap();
        assert_eq!(frame, reparsed);
        assert_eq!(reparsed.dlc(), 3);
    }

    /// Accessor coverage — every getter is pinned once.
    #[test]
    fn accessors() {
        let id = CanId::standard(0x123).unwrap();
        let frame = CanFrame::new(id, &[1]).unwrap();
        assert_eq!(frame.id(), id);
        assert_eq!(frame.data(), &[1]);
        assert_eq!(frame.dlc(), 1);
        assert!(!frame.is_remote());
        assert!(!frame.is_fd());
        assert!(!frame.brs());
        assert!(!frame.error_state_indicator());
    }
}
