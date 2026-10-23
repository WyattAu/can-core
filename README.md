# can-core
[![Estate integration](https://github.com/WyattAu/estate-integration/actions/workflows/ci.yml/badge.svg)](https://github.com/WyattAu/estate-integration/actions/workflows/ci.yml)

CAN 2.0 and CAN FD frame codec: identifier parsing, DLC handling, bit-timing
calculation, arbitration model.

**Layer:** L0: leaf · **Estate deps:** zero · **Runtime deps:** `thiserror`
only · `no_std` + `alloc` (CI exercises `thumbv7em-none-eabihf`).

## What's here

| Module | Contents |
|---|---|
| `id` | [`CanId`](https://docs.rs/can-core/latest/can_core/enum.CanId.html): 11-bit standard / 29-bit extended identifiers (`CanId::Standard(u11)` / `CanId::Extended(u29)`), range-validated constructors, SocketCAN id-word conversion |
| `frame` | `CanFrame`: zero-copy borrowed view over a payload; `parse`/`to_bytes` in SocketCAN wire format (`struct can_frame` = 16 B, `struct canfd_frame` = 72 B); the full DLC ↔ length table (classic 0–8, FD `0..8,12,16,20,24,32,48,64`) |
| `timing` | `BitTiming::calculate`: the standard Bosch bit-timing algorithm (prescaler search + segment split, sample-point targeting under Bosch segment limits) and SJA1000-style `btr()` register encoding |
| `arbitration` | `dominates`: the wire-level arbitration model: lower ids win, standard beats extended on a shared 11-bit prefix (dominant IDE vs recessive SRR), CAN FD unchanged at arbitration phase |
| `error` | `CanError`: the single typed, exhaustive error |

## Guarantees

- **Total parser**: `CanFrame::parse` never panics: bounds checked at every
  access, exhaustive typed errors, deny-level lints
  (`unwrap_used`/`expect_used`/`panic`/`indexing_slicing`) on the lib target,
  a cargo-fuzz target (`frame_parse`), 30 s CI fuzz smoke, miri over the
  suites.
- **Zero-copy**: `parse` borrows the payload straight out of the input
  buffer; nothing is copied or allocated until `to_bytes`.
- **In-range ids by construction**: validating constructors reject
  out-of-range identifiers (`0x7FF` / `0x1FFF_FFFF` caps); crate entry
  points re-validate (defense in depth).
- **FD discipline**: remote frames carry no payload; CAN FD frames cannot be
  remote; BRS/ESI only on FD; FD lengths must be DLC-representable.

## Example

```rust
use can_core::{CanFrame, CanId};

// CAN FD round-trip:
let id = CanId::extended(0x18FF_0102).unwrap();
let frame = CanFrame::fd(id, &[0xDE, 0xAD, 0xBE, 0xEF], true, false).unwrap();
let bytes = frame.to_bytes();
assert_eq!(bytes.len(), 72);
assert_eq!(CanFrame::parse(&bytes).unwrap(), frame);

// Bit timing: 500 kbit/s at a 48 MHz clock, CiA 87.5 % sample point.
let timing = can_core::timing::BitTiming::calculate(500_000, 48_000_000).unwrap();
assert_eq!(timing.btr().map(|b| b >> 8), Some(0x40)); // BTR0: SJW 1, BRP 1
```

## Tests

`cargo test` runs unit + integration suites, doctests, and a 500-case
proptest battery (classic/FD/remote round-trips, parse totality, DLC-table
bijection). Known-value bit-timing vectors pin the Bosch algorithm (CANopen
16 Tq / 87.5 % configurations). `cargo fuzz run frame_parse` covers the
parser.

## License

MIT OR Apache-2.0: see LICENSE-MIT and LICENSE-APACHE.
