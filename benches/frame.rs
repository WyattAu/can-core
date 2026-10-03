//! Criterion bench: frame `parse`/`to_bytes` hot paths.

// Test/bench file: criterion_group! generates undocumented items, and bench
// bodies unwrap known-good fixtures by design.
#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use can_core::{CanFrame, CanId};
use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;

fn bench_classic(c: &mut Criterion) {
    let id = CanId::standard(0x123).unwrap();
    let payload = [0x11u8; 8];
    let frame = CanFrame::new(id, &payload).unwrap();
    let bytes = frame.to_bytes();
    c.bench_function("classic_to_bytes", |b| {
        b.iter(|| black_box(&frame).to_bytes());
    });
    c.bench_function("classic_parse", |b| {
        b.iter(|| CanFrame::parse(black_box(&bytes)).unwrap());
    });
}

fn bench_fd(c: &mut Criterion) {
    let id = CanId::extended(0x18FF_0102).unwrap();
    let payload = [0x22u8; 64];
    let frame = CanFrame::fd(id, &payload, true, false).unwrap();
    let bytes = frame.to_bytes();
    c.bench_function("fd_to_bytes", |b| {
        b.iter(|| black_box(&frame).to_bytes());
    });
    c.bench_function("fd_parse", |b| {
        b.iter(|| CanFrame::parse(black_box(&bytes)).unwrap());
    });
}

fn bench_timing(c: &mut Criterion) {
    c.bench_function("bit_timing_solve", |b| {
        b.iter(|| {
            can_core::timing::BitTiming::calculate(black_box(500_000), black_box(48_000_000))
        });
    });
}

criterion_group!(benches, bench_classic, bench_fd, bench_timing);
criterion_main!(benches);
