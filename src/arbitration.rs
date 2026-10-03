//! Arbitration — why lower identifiers win, and what CAN FD changes (nothing
//! where it matters).
//!
//! # The rule
//!
//! CAN is a multi-master bus with bitwise arbitration: every transmitter
//! echoes the bus while sending its identifier and **loses the bus at the
//! first bit that does not match** — a node sending a recessive bit (1) while
//! another node drives a dominant bit (0) sees the dominant level and backs
//! off. Therefore the frame whose identifier presents a dominant bit earliest
//! wins: **numerically lower identifiers always beat higher ones** (when
//! widths agree). Arbitration is non-destructive — the winner's bits are on
//! the bus exactly as if no contention had occurred.
//!
//! # The 11-bit vs 29-bit subtlety
//!
//! Extended (29-bit) frames do not simply compare as 29-bit numbers against
//! standard (11-bit) frames. On the wire the identifier portion is
//!
//! * standard: `ID10..ID0`, then the **IDE bit = 0** (dominant);
//! * extended: `ID28..ID18`, then **SRR = 1** (recessive), then **IDE = 1**
//!   (recessive), then `ID17..ID0`.
//!
//! When a standard and an extended frame share the same first 11 id bits,
//! the standard frame drives a dominant IDE while the extended frame sends
//! recessive SRR — **the standard frame wins** even if its numeric id is
//! *higher* than the extended frame's. [`dominates`] models exactly this
//! wire-level comparison.
//!
//! # The CAN FD "exception"
//!
//! CAN FD does not change arbitration. FD frames arbitrate the identifier at
//! the **nominal** bit rate exactly like classic frames; the bit-rate switch
//! (BRS) happens only *after* arbitration is won, at the start of the data
//! phase. Contention between a classic and an FD frame (both at nominal
//! rate, FD frames share the id format) therefore resolves by the same
//! [`dominates`] rule — and an FD frame transmitting at the faster data
//! phase never corrupts arbitration, because arbitration is already over.
//! The one real FD-vs-classic hazard — an FD frame's CRC/stuff bits being
//! misread by a classic listener — is an error-frame concern, not an
//! arbitration one.

use crate::id::CanId;
use alloc::boxed::Box;

/// The identifier bits a node puts on the wire during arbitration, as
/// `(dominant == false)` booleans, in transmission order.
///
/// * standard: `ID10..ID0` MSB-first, then IDE = 0 (dominant)
/// * extended: `ID28..ID18`, SRR = 1, IDE = 1, then `ID17..ID0` MSB-first
fn transmitted_bits(id: CanId) -> impl Iterator<Item = bool> {
    match id {
        CanId::Standard(v) => Box::new(
            (0..11)
                .rev()
                .map(move |bit| (u32::from(v) >> bit) & 1 == 1)
                .chain(core::iter::once(false)), // IDE: dominant
        ) as Box<dyn Iterator<Item = bool>>,
        CanId::Extended(v) => Box::new(
            (18..29)
                .rev()
                .map(move |bit| (v >> bit) & 1 == 1)
                .chain(core::iter::once(true)) // SRR: recessive
                .chain(core::iter::once(true)) // IDE: recessive
                .chain((0..18).rev().map(move |bit| (v >> bit) & 1 == 1)),
        ) as Box<dyn Iterator<Item = bool>>,
    }
}

/// `true` when frame `a` wins arbitration against frame `b` — the wire-level
/// model: `a` transmits, `b` yields at the first bit where `a` drives
/// dominant while `b` sends recessive.
///
/// Two frames with the *same* transmitted id sequence (identical ids) never
/// dominate each other — that contention is a bus error, and this function
/// reports `false` for it.
///
/// ```
/// use can_core::arbitration::dominates;
/// use can_core::CanId;
///
/// let low = CanId::standard(0x100).unwrap();
/// let high = CanId::standard(0x200).unwrap();
/// assert!(dominates(low, high));
/// assert!(!dominates(high, low));
///
/// // The 11-bit vs 29-bit subtlety: a standard frame beats an extended
/// // frame that shares its first 11 bits (0x7F0 here) — even at a lower
/// // number for the extended frame.
/// let std = CanId::standard(0x7F0).unwrap();      // id: 0x7F0
/// let ext = CanId::extended(0x1FC0_0001).unwrap(); // top 11 bits: 0x7F0
/// assert!(dominates(std, ext));
/// ```
#[must_use]
pub fn dominates(a: CanId, b: CanId) -> bool {
    let mut a_bits = transmitted_bits(a);
    let mut b_bits = transmitted_bits(b);
    core::iter::from_fn(|| Some((a_bits.next()?, b_bits.next()?)))
        .find(|(x, y)| x != y)
        .is_some_and(|(x, y)| !x && y)
}

#[cfg(test)]
mod tests {
    // Tests exercise edge values and index fixed-size buffers directly:
    // unwrap/expect/indexing are lifted at test scope by design.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;
    use alloc::vec::Vec;

    #[test]
    fn lower_id_wins_among_equal_widths() {
        for (a, b) in [
            (0x000, 0x001),
            (0x100, 0x200),
            (0x7FE, 0x7FF),
            (0x123, 0x456),
        ] {
            let a = CanId::standard(a).unwrap();
            let b = CanId::standard(b).unwrap();
            assert!(dominates(a, b), "{a:?} should dominate {b:?}");
            assert!(!dominates(b, a));
        }
        // Extended frames obey the numeric rule among themselves.
        let lo = CanId::extended(0x0000_0100).unwrap();
        let hi = CanId::extended(0x0000_0200).unwrap();
        assert!(dominates(lo, hi));
        assert!(!dominates(hi, lo));
    }

    #[test]
    fn identical_ids_dominate_nothing() {
        let a = CanId::standard(0x123).unwrap();
        assert!(!dominates(a, a));
        let e = CanId::extended(0x0123_4567).unwrap();
        assert!(!dominates(e, e));
    }

    #[test]
    fn standard_beats_extended_on_shared_prefix() {
        // Same top 11 bits (0x7F0): the standard frame's dominant IDE beats
        // the extended frame's recessive SRR.
        let std = CanId::standard(0x7F0).unwrap();
        let ext = CanId::extended(0x1FC0_0001).unwrap();
        assert_eq!(ext.raw() >> 18, 0x7F0); // shared prefix, as claimed
        assert!(dominates(std, ext));
        assert!(!dominates(ext, std));
    }

    #[test]
    fn extended_wins_on_genuinely_lower_prefix() {
        // Extended top-11 = 0x100 vs standard 0x200: at bit 8 the extended
        // frame drives a dominant 0 first — it wins regardless of format.
        let std = CanId::standard(0x200).unwrap();
        let ext = CanId::extended(0x0100_0000).unwrap();
        assert!(dominates(ext, std));
        assert!(!dominates(std, ext));
    }

    /// Transmitted-bit sequences have the documented shape.
    #[test]
    fn wire_sequence_shape() {
        // Standard 0x000: eleven zeros then a dominant IDE — all zeros.
        assert!(transmitted_bits(CanId::Standard(0)).all(|bit| !bit));
        // Standard 0x7FF: eleven ones then dominant IDE (0).
        let bits: Vec<_> = transmitted_bits(CanId::Standard(0x7FF)).collect();
        assert_eq!(bits.len(), 12);
        assert!(bits[..11].iter().all(|bit| *bit));
        assert!(!bits[11]);
        // Extended 0x1FFF_FFFF: 29 ones wrapped by recessive SRR/IDE = 31
        // ones.
        let bits: Vec<_> = transmitted_bits(CanId::Extended(0x1FFF_FFFF)).collect();
        assert_eq!(bits.len(), 31);
        assert!(bits.iter().all(|bit| *bit));
        // Extended 0: dominant at the very first bit.
        assert!(!transmitted_bits(CanId::Extended(0)).next().unwrap());
    }
}
