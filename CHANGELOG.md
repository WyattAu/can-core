# Changelog

All notable changes to this project will be documented in this file.
Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning:
[SemVer](https://semver.org/).

## [0.1.0]: 2026-10-03

Initial release: L0 leaf, zero estate deps, `no_std` + `alloc`.

### Added

- **Identifiers**: `CanId::Standard(u16)` / `CanId::Extended(u32)` (11-bit /
  29-bit), range-validated constructors (`standard`, `extended`,
  `from_raw_socketcan`), SocketCAN id-word round-trips.
- **Frames**: `CanFrame`: zero-copy borrowed view; `parse`/`to_bytes` over
  the SocketCAN `struct can_frame` (16 B) / `struct canfd_frame` (72 B)
  formats; constructors `new` (classic data), `fd` (CAN FD with BRS/ESI),
  `remote` (RTR), and the fully validated `from_parts`; `len8_dlc`
  refinement support on classic frames.
- **DLC table**: `dlc_to_len`/`len_to_dlc`: classic 0–8, CAN FD
  `0,1,2,3,4,5,6,7,8,12,16,20,24,32,48,64`, with typed errors for
  non-representable lengths.
- **Bit timing**: `timing::BitTiming::calculate`/`calculate_with`: the
  Bosch prescaler + segment-split search under Bosch segment limits, CiA
  default sample points (87.5 % / 75 %), SJA1000-style `btr()` encoding,
  `actual_baudrate` sanity check.
- **Arbitration**: `arbitration::dominates`: wire-level model including the
  standard-beats-extended SRR/IDE rule; documented CAN FD arbitration-phase
  behavior (unchanged; BRS applies after arbitration).
- **Errors**: `CanError`: typed, exhaustive (semver-minor additions by
  design).
- **Gates**: 500-case proptest battery, cargo-fuzz target `frame_parse`
  with committed corpus, miri, embedded (`thumbv7em-none-eabihf`), tier-a
  shared CI pinned to engineering-standards `main`.

[0.1.0]: https://github.com/WyattAu/can-core/releases/tag/v0.1.0
