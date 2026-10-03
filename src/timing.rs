//! CAN bit-timing — the standard Bosch calculation from baudrate and clock.
//!
//! A bit time is divided into time quanta (Tq): one **synchronisation
//! segment** (always 1 Tq), a **propagation segment**, **phase segment 1**
//! (the sample point falls at the end of phase segment 1), and **phase
//! segment 2**. With a prescaler (BRP) dividing the input clock, the bit
//! time is
//!
//! ```text
//! Tq_per_bit = clock_hz / (prescaler × baudrate)
//!            = 1 (sync) + prop_seg + phase_seg1 + phase_seg2
//! ```
//!
//! [`BitTiming::calculate`] searches prescalers and segment splits for the
//! configuration whose sample point lands closest to the target — the
//! canonical algorithm Linux (`can_update_spt`) and every Bosch-spec-derived
//! tool uses — under the Bosch segment limits (prop 1–8, phase1 1–8, phase2
//! 2–8, SJW 1–4, giving 4–25 Tq per bit). Ties prefer the smaller phase
//! segment 2 (the higher sample point) and the smaller prescaler.
//!
//! [`BitTiming::btr`] renders the result as an SJA1000-style BTR register
//! pair: `BTR0 = (SJW−1) << 6 | (BRP−1)`, `BTR1 = (PHASE_SEG2−1) |
//! ((PROP_SEG+PHASE_SEG1−1) << 4)`.

use crate::error::CanError;

/// Default sample point for bit rates ≥ 800 kbit/s (`CiA` recommendation).
pub const SAMPLE_POINT_HIGH_SPEED: f64 = 0.750;
/// Default sample point below 800 kbit/s (`CiA` recommendation — 87.5 %).
pub const SAMPLE_POINT_DEFAULT: f64 = 0.875;

/// Bosch CAN 2.0 segment limits.
const PROP_SEG_MAX: u16 = 8;
const PHASE_SEG1_MAX: u16 = 8;
const PHASE_SEG2_MIN: u16 = 2;
const PHASE_SEG2_MAX: u16 = 8;
const PHASE_SEG1_MIN: u16 = 2; // prop + phase1 ≥ 2
const TOTAL_MAX: u16 = 1 + PROP_SEG_MAX + PHASE_SEG1_MAX + PHASE_SEG2_MAX; // 25
const SJW_MAX: u16 = 4;

/// The four programmable bit-timing segments, in time quanta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitSegments {
    /// Propagation segment (1–8 Tq).
    pub prop_seg: u16,
    /// Phase segment 1 (1–8 Tq); the sample point sits at its end.
    pub phase_seg1: u16,
    /// Phase segment 2 (2–8 Tq).
    pub phase_seg2: u16,
    /// Synchronisation jump width (1–4 Tq).
    pub sjw: u16,
}

impl BitSegments {
    /// Total bit time in Tq: sync (1) + all segments.
    #[must_use]
    pub const fn total_tq(&self) -> u16 {
        1 + self.prop_seg + self.phase_seg1 + self.phase_seg2
    }
}

/// A solved bit-timing configuration for one baudrate/clock pair.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BitTiming {
    /// The achieved bit rate in bit/s.
    pub baudrate: u32,
    /// Sample point as a fraction of the bit time, `0.0..=1.0`.
    pub sample_point: f64,
    /// The prescaler (BRP) applied to the input clock.
    pub prescaler: u32,
    /// The segment values, in time quanta.
    pub segments: BitSegments,
}

/// Parameters for [`BitTiming::calculate_with`].
#[derive(Debug, Clone, Copy)]
pub struct BitTimingParams {
    /// CAN controller input clock, in Hz.
    pub clock_hz: u32,
    /// Target sample point (`Some` to override, `None` for the `CiA` default:
    /// 87.5 % below 800 kbit/s, 75 % at or above).
    pub sample_point: Option<f64>,
    /// Largest prescaler to consider (hardware BRP width; default 1024).
    pub max_prescaler: u32,
}

impl BitTimingParams {
    /// Parameters for `clock_hz` with the default sample point and prescaler
    /// limit.
    #[must_use]
    pub const fn new(clock_hz: u32) -> Self {
        Self {
            clock_hz,
            sample_point: None,
            max_prescaler: 1024,
        }
    }
}

/// CiA-recommended default sample point for a bit rate.
#[must_use]
pub fn default_sample_point(baudrate: u32) -> f64 {
    if baudrate >= 800_000 {
        SAMPLE_POINT_HIGH_SPEED
    } else {
        SAMPLE_POINT_DEFAULT
    }
}

impl BitTiming {
    /// Solves the standard bit-timing algorithm for `baudrate` driven by
    /// `clock_hz`, using the `CiA` default sample point.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidTiming`] for a zero baudrate/clock or when no
    /// configuration under the Bosch segment limits can achieve the
    /// baudrate.
    pub fn calculate(baudrate: u32, clock_hz: u32) -> Result<Self, CanError> {
        Self::calculate_with(BitTimingParams::new(clock_hz), baudrate)
    }

    /// Solves the bit-timing algorithm with explicit parameters.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidTiming`] for a zero baudrate/clock, a sample point
    /// outside `(0, 1)`, or no achievable configuration.
    pub fn calculate_with(params: BitTimingParams, baudrate: u32) -> Result<Self, CanError> {
        if baudrate == 0 {
            return Err(CanError::InvalidTiming("baudrate must be non-zero"));
        }
        if params.clock_hz == 0 {
            return Err(CanError::InvalidTiming("clock_hz must be non-zero"));
        }
        let target = params
            .sample_point
            .unwrap_or_else(|| default_sample_point(baudrate));
        if !(target > 0.0 && target < 1.0) {
            return Err(CanError::InvalidTiming("sample_point must lie in (0, 1)"));
        }
        if params.max_prescaler == 0 {
            return Err(CanError::InvalidTiming("max_prescaler must be non-zero"));
        }

        let mut best: Option<(f64, u32, u16, u16)> = None; // (error, brp, tseg1_total, tseg2)
        for brp in 1..=params.max_prescaler {
            let scaled = u64::from(brp) * u64::from(baudrate);
            if scaled > u64::from(params.clock_hz) {
                break; // larger brp only makes it worse
            }
            // scaled ≤ clock by the break above — no truncation.
            #[allow(clippy::cast_possible_truncation, clippy::cast_lossless)]
            let scaled = scaled as u32;
            if params.clock_hz % scaled != 0 {
                continue; // not an integer number of Tq — unusable
            }
            let total = params.clock_hz / scaled;
            if total > u32::from(TOTAL_MAX) {
                continue; // bit time too long even at this prescaler
            }
            // ≤ TOTAL_MAX ≤ 25 by the check above — no truncation.
            #[allow(clippy::cast_possible_truncation)]
            let total = total as u16;
            // tseg = tseg1_total + tseg2 (sync segment excluded).
            let tseg = total - 1;
            for tseg2 in PHASE_SEG2_MIN..=PHASE_SEG2_MAX {
                if tseg2 >= tseg {
                    break;
                }
                let tseg1_total = tseg - tseg2;
                if !(PHASE_SEG1_MIN..=PHASE_SEG1_MAX + PROP_SEG_MAX).contains(&tseg1_total) {
                    continue;
                }
                let sample_point = f64::from(total - tseg2) / f64::from(total);
                let error = (sample_point - target).abs();
                let better = match best {
                    None => true,
                    // Strict `<` keeps the first winner: smaller tseg2 (higher
                    // sample point) on ties, and the smallest brp (outer loop).
                    Some((best_error, _, _, _)) => error < best_error,
                };
                if better {
                    best = Some((error, brp, tseg1_total, tseg2));
                }
            }
        }

        let Some((_, brp, tseg1_total, tseg2)) = best else {
            return Err(CanError::InvalidTiming(
                "no configuration achieves this baudrate under the Bosch segment limits",
            ));
        };

        // Split tseg1 between propagation and phase 1: phase 1 takes half,
        // clamped to the Bosch 8-Tq cap (prop then carries the remainder;
        // both stay ≥ 1 for tseg1_total ∈ 2..=16).
        let phase_seg1 = (tseg1_total / 2).min(PHASE_SEG1_MAX);
        let prop_seg = tseg1_total - phase_seg1;
        let segments = BitSegments {
            prop_seg,
            phase_seg1,
            phase_seg2: tseg2,
            sjw: tseg2.min(SJW_MAX),
        };
        let sample_point = f64::from(tseg1_total + 1) / f64::from(tseg1_total + 1 + tseg2);
        Ok(Self {
            baudrate,
            sample_point,
            prescaler: brp,
            segments,
        })
    }

    /// The achieved bit rate implied by the configuration — sanity check
    /// that `prescaler × baudrate` divides the clock exactly.
    ///
    /// # Errors
    ///
    /// [`CanError::InvalidTiming`] when the clock passed here cannot produce
    /// an integer number of Tq for this configuration.
    pub fn actual_baudrate(&self, clock_hz: u32) -> Result<u32, CanError> {
        let tq = u32::from(self.segments.total_tq());
        let dividend = u64::from(clock_hz);
        let divisor = u64::from(self.prescaler) * u64::from(tq);
        if divisor == 0 || dividend % divisor != 0 {
            return Err(CanError::InvalidTiming(
                "clock does not divide into an integer Tq count",
            ));
        }
        // Quotient ≤ dividend ≤ u32::MAX — no truncation.
        #[allow(clippy::cast_possible_truncation)]
        Ok((dividend / divisor) as u32)
    }

    /// The SJA1000-style BTR register pair, `BTR0 << 8 | BTR1`:
    /// `BTR0 = (SJW−1) << 6 | (BRP−1)`, `BTR1 = (PHASE_SEG2−1) |
    /// ((PROP_SEG+PHASE_SEG1−1) << 4)` with SAM = 0.
    ///
    /// `None` when the prescaler exceeds the 6-bit BTR0 field (BRP > 64) —
    /// the segment fields can never overflow under the Bosch limits.
    #[must_use]
    pub fn btr(&self) -> Option<u16> {
        if self.prescaler > 64 {
            return None;
        }
        // ≤ 64 by the check above — no truncation.
        #[allow(clippy::cast_possible_truncation)]
        let brp_field = (self.prescaler - 1) as u16;
        let sjw_field = self.segments.sjw - 1;
        let btr0 = (sjw_field << 6) | brp_field;
        let tseg1_total = self.segments.prop_seg + self.segments.phase_seg1;
        let btr1 = (self.segments.phase_seg2 - 1) | ((tseg1_total - 1) << 4);
        Some((btr0 << 8) | btr1)
    }
}

#[cfg(test)]
mod tests {
    // Tests exercise edge values and index fixed-size buffers directly:
    // unwrap/expect/indexing are lifted at test scope by design.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
    use super::*;

    /// Canonical `CANopen` (`CiA`) table values at an 8 MHz clock: 125/250/500
    /// kbit/s all land on 16 Tq with an exact 87.5 % sample point.
    #[test]
    fn cia_8mhz_classic_table() {
        // 125 kbit/s: BRP 4, 16 Tq, SP 14/16 = 87.5 %.
        let t = BitTiming::calculate(125_000, 8_000_000).unwrap();
        assert_eq!(t.prescaler, 4);
        assert_eq!(t.segments.total_tq(), 16);
        assert!((t.sample_point - 0.875).abs() < 1e-12);
        assert_eq!(
            t.segments,
            BitSegments {
                prop_seg: 7,
                phase_seg1: 6,
                phase_seg2: 2,
                sjw: 2
            }
        );
        assert_eq!(t.btr(), Some(0x43C1));
        assert_eq!(t.actual_baudrate(8_000_000), Ok(125_000));

        // 250 kbit/s: BRP 2 — same segments otherwise.
        let t = BitTiming::calculate(250_000, 8_000_000).unwrap();
        assert_eq!(t.prescaler, 2);
        assert_eq!(t.btr(), Some(0x41C1));

        // 500 kbit/s: BRP 1.
        let t = BitTiming::calculate(500_000, 8_000_000).unwrap();
        assert_eq!(t.prescaler, 1);
        assert_eq!(t.btr(), Some(0x40C1));
    }

    /// 1 Mbit/s at 8 MHz takes the high-speed 75 % default: 8 Tq, BRP 1,
    /// exact sample point.
    #[test]
    fn high_speed_default_sample_point() {
        assert!((default_sample_point(1_000_000) - 0.750).abs() < 1e-12);
        assert!((default_sample_point(500_000) - 0.875).abs() < 1e-12);

        let t = BitTiming::calculate(1_000_000, 8_000_000).unwrap();
        assert_eq!(t.prescaler, 1);
        assert_eq!(t.segments.total_tq(), 8);
        assert!((t.sample_point - 0.750).abs() < 1e-12);
        assert_eq!(
            t.segments,
            BitSegments {
                prop_seg: 3,
                phase_seg1: 2,
                phase_seg2: 2,
                sjw: 2
            }
        );
        assert_eq!(t.btr(), Some(0x4041));
    }

    /// An explicit sample-point target overrides the default, and the search
    /// lands on the closest achievable point.
    #[test]
    fn explicit_sample_point() {
        // 500 kbit/s at 48 MHz: 96 Tq raw. The closest sample point to the
        // 87.5 % target is exact — 16 Tq at prescaler 6, tseg2 = 2.
        let t = BitTiming::calculate_with(
            BitTimingParams {
                clock_hz: 48_000_000,
                sample_point: Some(0.875),
                max_prescaler: 1024,
            },
            500_000,
        )
        .unwrap();
        assert_eq!(t.segments.total_tq(), 16);
        assert_eq!(t.prescaler, 6);
        assert!((t.sample_point - 0.875).abs() < 1e-12);
        assert_eq!(t.btr(), Some(0x45C1));

        // An unreachable-tight sample point still resolves to the closest
        // option: at 16 Tq the lowest achievable point is exactly 8/16.
        let t = BitTiming::calculate_with(
            BitTimingParams {
                clock_hz: 8_000_000,
                sample_point: Some(0.5),
                max_prescaler: 1024,
            },
            125_000,
        )
        .unwrap();
        assert_eq!(t.segments.total_tq(), 16);
        assert!((t.sample_point - 0.5).abs() < 1e-12);
        assert_eq!(t.segments.phase_seg2, 8);
        assert_eq!(t.segments.sjw, 4);
    }

    #[test]
    fn parameter_errors() {
        assert_eq!(
            BitTiming::calculate(0, 8_000_000),
            Err(CanError::InvalidTiming("baudrate must be non-zero"))
        );
        assert_eq!(
            BitTiming::calculate(500_000, 0),
            Err(CanError::InvalidTiming("clock_hz must be non-zero"))
        );
        assert_eq!(
            BitTiming::calculate_with(
                BitTimingParams {
                    clock_hz: 8_000_000,
                    sample_point: Some(1.0),
                    max_prescaler: 64
                },
                500_000
            ),
            Err(CanError::InvalidTiming("sample_point must lie in (0, 1)"))
        );
        assert_eq!(
            BitTiming::calculate_with(
                BitTimingParams {
                    clock_hz: 8_000_000,
                    sample_point: Some(0.875),
                    max_prescaler: 0
                },
                500_000
            ),
            Err(CanError::InvalidTiming("max_prescaler must be non-zero"))
        );
        // No integer Tq count can produce this rate under the segment limits:
        assert!(BitTiming::calculate(333_333, 8_000_000).is_err());
    }

    /// Prescalers past the 6-bit BTR0 field are reported, not wrapped.
    #[test]
    fn btr_overflow_is_none() {
        // 2 kbit/s at 8 MHz needs 4000 Tq / brp ≤ 25 → prescaler ≥ 160,
        // far past the 6-bit BTR0 field.
        let t = BitTiming::calculate(2_000, 8_000_000).unwrap();
        assert!(t.prescaler > 64);
        assert_eq!(t.btr(), None);
    }

    #[test]
    fn actual_baudrate_error() {
        let t = BitTiming::calculate(125_000, 8_000_000).unwrap();
        // 7 000 001 Hz does not divide into an integer Tq count (÷64).
        assert!(t.actual_baudrate(7_000_001).is_err());
    }
}
