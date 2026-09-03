//! Criterion benchmarks for the Urbix generation pipeline.
//!
//! Three groups that map to the performance objectives (§1, §7 / M7):
//!   1. single chunk (cold) — raw `chunk::generate_chunk` cost
//!   2. 100-chunk sweep — grid stress
//!   3. cache hit vs miss — `WorldEngine` with `ChunkCache`

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use urbix::config::WorldConfig;
use urbix::engine::WorldEngine;
use urbix::region::VoronoiDiagram;

fn bench_single_chunk(c: &mut Criterion) {
    let cfg = WorldConfig {
        seed: 445566,
        ..Default::default()
    };
    let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
    let mut group = c.benchmark_group("single_chunk");
    group.throughput(Throughput::Elements(
        (cfg.chunk_size as u64) * (cfg.chunk_size as u64),
    ));
    group.bench_function("generate_chunk/32x32", |b| {
        b.iter(|| {
            let buf = urbix::chunk::generate_chunk(black_box(0), black_box(0), &cfg, &voronoi);
            black_box(buf);
        })
    });
    group.finish();
}

fn bench_sweep(c: &mut Criterion) {
    let cfg = WorldConfig {
        seed: 445566,
        ..Default::default()
    };
    let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
    let mut group = c.benchmark_group("sweep_100");
    group.throughput(Throughput::Elements(100));
    group.bench_function("100_chunks/10x10", |b| {
        b.iter(|| {
            for cy in 0..10 {
                for cx in 0..10 {
                    let buf = urbix::chunk::generate_chunk(cx, cy, &cfg, &voronoi);
                    black_box(buf);
                }
            }
        })
    });
    group.finish();
}

fn bench_cache(c: &mut Criterion) {
    let mut group = c.benchmark_group("cache");
    group.bench_function("hit", |b| {
        let mut engine = WorldEngine::new(445566);
        // Prime cache.
        let _ = engine.generate_chunk(0, 0);
        b.iter(|| {
            let buf = engine.generate_chunk(black_box(0), black_box(0));
            black_box(buf);
        })
    });
    group.bench_function("miss", |b| {
        b.iter(|| {
            let mut engine = WorldEngine::new(black_box(445566));
            // Each iteration uses a fresh engine so every chunk is a miss.
            let buf = engine.generate_chunk(0, 0);
            black_box(buf);
        })
    });
    group.finish();
}

criterion_group!(benches, bench_single_chunk, bench_sweep, bench_cache);
criterion_main!(benches);
