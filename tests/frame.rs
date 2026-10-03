//! Round-trip, DLC-table, id-range, and FD-vs-classic constraint tests over
//! the public API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use can_core::{
    dlc_to_len, len_to_dlc, CanError, CanFrame, CanId, CLASSIC_MAX_LEN, FD_DLC_TO_LEN, FD_MAX_LEN,
};

#[test]
fn id_range_validation() {
    // 11-bit standard: max 0x7FF.
    assert!(CanId::standard(0x000).is_ok());
    assert!(CanId::standard(0x7FF).is_ok());
    assert_eq!(
        CanId::standard(0x800),
        Err(CanError::InvalidIdLength {
            max: 0x7FF,
            got: 0x800
        })
    );
    assert_eq!(
        CanId::standard(u16::MAX),
        Err(CanError::InvalidIdLength {
            max: 0x7FF,
            got: 0xFFFF
        })
    );

    // 29-bit extended: max 0x1FFFFFFF.
    assert!(CanId::extended(0x1FFF_FFFF).is_ok());
    assert_eq!(
        CanId::extended(0x2000_0000),
        Err(CanError::InvalidIdLength {
            max: 0x1FFF_FFFF,
            got: 0x2000_0000
        })
    );
}

#[test]
fn dlc_table_completeness() {
    // The FD table, verbatim — the contract of the crate.
    assert_eq!(
        FD_DLC_TO_LEN,
        [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32, 48, 64]
    );
    // Every entry round-trips through both directions.
    for (dlc, len) in FD_DLC_TO_LEN.iter().enumerate() {
        // dlc < 16 by construction — no truncation.
        #[allow(clippy::cast_possible_truncation)]
        let dlc = dlc as u8;
        assert_eq!(dlc_to_len(dlc).unwrap(), *len);
        assert_eq!(len_to_dlc(*len).unwrap(), dlc);
    }
    // DLCs outside 0..=15 are rejected; lengths outside the table too.
    assert_eq!(dlc_to_len(16), Err(CanError::InvalidDlc(16)));
    assert_eq!(dlc_to_len(255), Err(CanError::InvalidDlc(255)));
    for bad in [9, 10, 11, 13, 14, 15, 17, 33, 63, 65] {
        assert!(matches!(len_to_dlc(bad), Err(CanError::DataTooLong { .. })));
    }
}

#[test]
fn classic_roundtrip_parse_to_bytes() {
    let id = CanId::standard(0x123).unwrap();
    let frame = CanFrame::new(id, &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]).unwrap();
    let bytes = frame.to_bytes();
    assert_eq!(bytes.len(), 16);
    assert_eq!(CanFrame::parse(&bytes).unwrap(), frame);
    // Zero-payload and single-byte edge:
    let empty = CanFrame::new(id, &[]).unwrap();
    assert_eq!(CanFrame::parse(&empty.to_bytes()).unwrap(), empty);
}

#[test]
fn fd_roundtrip_parse_to_bytes() {
    let id = CanId::extended(0x18FF_0102).unwrap();
    for dlc in 0_u8..=15 {
        let len = dlc_to_len(dlc).unwrap();
        let payload = vec![0xA5_u8; len];
        let frame = CanFrame::fd(id, &payload, dlc % 2 == 0, dlc % 3 == 0).unwrap();
        assert_eq!(frame.dlc(), dlc);
        assert_eq!(frame.data().len(), len.min(FD_MAX_LEN));
        let bytes = frame.to_bytes();
        let reparsed = CanFrame::parse(&bytes).unwrap();
        assert_eq!(reparsed, frame);
        assert!(reparsed.is_fd());
    }
}

#[test]
fn remote_frame_roundtrip() {
    let id = CanId::standard(0x456).unwrap();
    for dlc in 0_u8..=15 {
        let frame = CanFrame::remote(id, dlc).unwrap();
        let bytes = frame.to_bytes();
        let reparsed = CanFrame::parse(&bytes).unwrap();
        assert_eq!(reparsed, frame);
        assert!(reparsed.is_remote());
        assert_eq!(reparsed.data(), &[]);
        assert_eq!(reparsed.dlc(), dlc);
    }
}

#[test]
fn fd_vs_classic_constraints() {
    let id = CanId::standard(0x123).unwrap();
    let nine = [0u8; 9];
    let twelve = [0u8; 12];
    let sixty_five = [0u8; 65];

    // Classic: 0..=8 bytes only; FD flags forbidden.
    assert!(CanFrame::new(id, &[]).is_ok());
    assert_eq!(
        CanFrame::new(id, &nine),
        Err(CanError::DataTooLong {
            max: CLASSIC_MAX_LEN,
            got: 9
        })
    );
    assert_eq!(
        CanFrame::from_parts(id, &[], 0, false, false, true, false),
        Err(CanError::FdBrsOnClassic)
    );
    assert_eq!(
        CanFrame::from_parts(id, &[], 0, false, false, false, true),
        Err(CanError::FdBrsOnClassic)
    );

    // FD: the classic cap does not apply, but lengths must be table entries.
    assert!(CanFrame::fd(id, &twelve, false, false).is_ok());
    assert!(CanFrame::fd(id, &[0u8; 64], false, false).is_ok());
    assert_eq!(
        CanFrame::fd(id, &nine, false, false),
        Err(CanError::DataTooLong { max: 8, got: 9 })
    );
    assert_eq!(
        CanFrame::fd(id, &sixty_five, false, false),
        Err(CanError::DataTooLong {
            max: FD_MAX_LEN,
            got: 65
        })
    );

    // FD frames cannot be remote — not even without payload:
    assert_eq!(
        CanFrame::from_parts(id, &[], 0, true, true, false, false),
        Err(CanError::RemoteFrameWithData)
    );
    // A remote frame cannot carry data:
    assert_eq!(
        CanFrame::from_parts(id, &[0x01], 1, true, false, false, false),
        Err(CanError::RemoteFrameWithData)
    );
}

#[test]
fn parse_rejects_wrong_sizes() {
    assert!(matches!(
        CanFrame::parse(&[0u8; 0]),
        Err(CanError::BufferTooShort { need: 16, have: 0 })
    ));
    assert!(matches!(
        CanFrame::parse(&[0u8; 15]),
        Err(CanError::BufferTooShort { .. })
    ));
    for size in [17, 32, 71, 73, 128] {
        assert!(matches!(
            CanFrame::parse(&vec![0u8; size]),
            Err(CanError::DataTooLong { .. })
        ));
    }
    // Both canonical sizes parse (as classic / FD respectively):
    assert!(CanFrame::parse(&[0u8; 16]).is_ok());
    assert!(CanFrame::parse(&[0u8; 72]).is_ok());
}

#[test]
fn error_display_covers_every_variant() {
    let cases: Vec<(CanError, &str)> = vec![
        (
            CanError::InvalidIdLength {
                max: 0x7FF,
                got: 0x800,
            },
            "identifier 0x800 exceeds its width limit 0x7FF",
        ),
        (
            CanError::DataTooLong { max: 8, got: 9 },
            "payload of 9 bytes exceeds the 8-byte maximum for this frame type",
        ),
        (
            CanError::InvalidDlc(16),
            "invalid DLC 16: expected 0..=15 (0..=8 for classic frames)",
        ),
        (
            CanError::FdBrsOnClassic,
            "FD-only flag (BRS/ESI) set on a classic CAN frame",
        ),
        (
            CanError::RemoteFrameWithData,
            "remote frame carrying data bytes, or FD frame marked remote",
        ),
        (
            CanError::BufferTooShort { need: 16, have: 4 },
            "buffer of 4 bytes is too short: need 16",
        ),
        (
            CanError::InvalidTiming("baudrate must be non-zero"),
            "invalid bit-timing parameters: baudrate must be non-zero",
        ),
    ];
    for (error, expected) in cases {
        assert_eq!(error.to_string(), expected);
        // Debug formatting is part of the contract too.
        assert_ne!(format!("{error:?}"), "");
    }
}

#[test]
fn timing_known_values_bosch() {
    use can_core::timing::{BitSegments, BitTiming};

    // The CANopen/CiA-classic values derived from the Bosch algorithm at an
    // 8 MHz crystal: 125 kbit/s → 16 Tq (BRP 4), sample point exactly 87.5 %.
    // BTR0 = (SJW−1)<<6 | (BRP−1) = 0x43; BTR1 = (PHASE_SEG2−1) | (TSEG1−1)<<4
    // = 1 | 12<<4 = 0xC1.
    let t125 = BitTiming::calculate(125_000, 8_000_000).unwrap();
    assert_eq!(
        t125,
        BitTiming {
            baudrate: 125_000,
            sample_point: 0.875,
            prescaler: 4,
            segments: BitSegments {
                prop_seg: 7,
                phase_seg1: 6,
                phase_seg2: 2,
                sjw: 2
            },
        }
    );
    assert_eq!(t125.btr(), Some(0x43C1));

    // 1 Mbit/s at 8 MHz: high-speed default 75 % → 8 Tq, BRP 1, exact.
    let t1m = BitTiming::calculate(1_000_000, 8_000_000).unwrap();
    assert_eq!(t1m.segments.total_tq(), 8);
    assert!((t1m.sample_point - 0.75).abs() < 1e-12);
    assert_eq!(t1m.btr(), Some(0x4041));
}
