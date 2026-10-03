//! Fuzz target — `CanFrame::parse` totality: arbitrary bytes produce either
//! a frame or a typed error, never a panic. Round-trip consistency is
//! asserted for anything that parses.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Ok(frame) = can_core::CanFrame::parse(data) {
        // Whatever parses must survive a round-trip through to_bytes.
        let bytes = frame.to_bytes();
        let reparsed = can_core::CanFrame::parse(&bytes);
        assert_eq!(Ok(frame), reparsed, "to_bytes/parse round-trip diverged");
    }
});
