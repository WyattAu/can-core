# Security Policy — can-core

## Supported versions

| Version | Supported |
|---------|-----------|
| 0.1.x   | ✅        |

## Reporting a vulnerability

Report privately via [GitHub security advisories] for this repository, or
email **wyatt_au@protonmail.com**. Do **not** open a public issue for
security reports.

You will receive an acknowledgement within **72 hours**. Coordinated
disclosure: we ask for up to 90 days before public disclosure while a
patch ships.

[GitHub security advisories]: https://github.com/WyattAu/can-core/security/advisories/new

## Threat model

**I/O surface: untrusted input.** `can-core` parses frames that originate
from CAN buses, SocketCAN sockets, log captures, and other untrusted
sources. The trust boundary is the *contents* of every `&[u8]` handed to
`CanFrame::parse`. Callers control where the bytes come from; can-core
guarantees safety of the *decode* regardless.

| ID | Threat | Mitigation |
|----|--------|------------|
| T1 | Malformed/truncated frames drive the parser out of bounds or into a panic (remote crash on hostile bus capture) | Totality by construction: bounds checked at every access (`get()`-only access; `unwrap`/`expect`/`panic`/`indexing_slicing` denied at the lib target), exhaustive `CanError`, cargo-fuzz target `frame_parse`, 30 s CI smoke, miri over the suites |
| T2 | Out-of-range identifiers silently truncated, hiding bus misconfiguration | Validating constructors reject (`InvalidIdLength`) — ids are never silently masked |
| T3 | FD/classic confusion (BRS/ESI on classic, remote FD frames) corrupts controller state | Typed rejections (`FdBrsOnClassic`, `RemoteFrameWithData`); the wire formats are distinguished by exact buffer size |
| T4 | Malicious dependency substitution in the build chain | cargo-deny (advisories/licenses/bans) and cargo-vet gates in CI; supply-chain audits committed |

## Scope

`can-core` is a codec and model library: it performs no I/O, spawns no
threads, and never executes attacker-controlled code. Memory-safety bugs
would require an `unsafe` block — the crate contains none
(`unsafe_code = "deny"`).
