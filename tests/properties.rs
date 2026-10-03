// Test file: fixed-size table indexing and strategic unwraps are by design.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! proptest: frame `to_bytes`/`parse` round-trips are exact, and `parse` is
//! total — arbitrary input never panics. 500 cases per property.

use can_core::{dlc_to_len, len_to_dlc, CanFrame, CanId, FD_DLC_TO_LEN};
use proptest::prelude::*;

/// Standard or extended identifiers, in range by construction.
fn can_ids() -> impl Strategy<Value = CanId> {
    prop_oneof![
        // v ≤ 0x7FF by the range above — no truncation.
        #[allow(clippy::cast_possible_truncation)]
        (0u32..=0x7FF_u32).prop_map(|v| CanId::Standard(v as u16)),
        (0u32..=0x1FFF_FFFF_u32).prop_map(CanId::Extended),
    ]
}

fn payloads(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..=max)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Classic data frames: `parse(to_bytes(x)) == x`, wire size 16.
    #[test]
    fn classic_roundtrip(id in can_ids(), data in payloads(8)) {
        let frame = CanFrame::new(id, &data).unwrap();
        let bytes = frame.to_bytes();
        prop_assert_eq!(bytes.len(), 16);
        prop_assert_eq!(CanFrame::parse(&bytes).unwrap(), frame);
    }

    /// CAN FD data frames across every DLC: exact round-trip, wire size 72.
    #[test]
    fn fd_roundtrip(
        id in can_ids(),
        dlc in 0u8..=15,
        brs in any::<bool>(),
        esi in any::<bool>(),
        filler in any::<u8>(),
    ) {
        let data = vec![filler; dlc_to_len(dlc).unwrap()];
        let frame = CanFrame::fd(id, &data, brs, esi).unwrap();
        prop_assert_eq!(frame.dlc(), dlc);
        let bytes = frame.to_bytes();
        prop_assert_eq!(bytes.len(), 72);
        prop_assert_eq!(CanFrame::parse(&bytes).unwrap(), frame);
    }

    /// Remote frames: no payload, DLC survives, RTR flag on the wire.
    #[test]
    fn remote_roundtrip(id in can_ids(), dlc in 0u8..=15) {
        let frame = CanFrame::remote(id, dlc).unwrap();
        let bytes = frame.to_bytes();
        prop_assert_eq!(bytes.len(), 16);
        let reparsed = CanFrame::parse(&bytes).unwrap();
        prop_assert!(reparsed.is_remote());
        prop_assert!(reparsed.data().is_empty());
        prop_assert_eq!(reparsed.dlc(), dlc);
        prop_assert_eq!(reparsed, frame);
    }

    /// Totality: arbitrary byte soup never panics `parse`.
    #[test]
    fn parse_is_total(junk in prop::collection::vec(any::<u8>(), 0..80)) {
        let _ = CanFrame::parse(&junk);
    }

    /// The DLC table is a bijection between 0..=15 and its length set.
    #[test]
    fn dlc_table_is_bijection(dlc in 0u8..=15) {
        let len = dlc_to_len(dlc).unwrap();
        prop_assert_eq!(len, FD_DLC_TO_LEN[usize::from(dlc)]);
        prop_assert_eq!(len_to_dlc(len).unwrap(), dlc);
    }
}
