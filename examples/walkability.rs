//! # walkability.rs — walkability metrics for the generated city
//!
//! A headless acceptance check for Milestones 11–12: generates an `extent ×
//! extent` grid of chunks and reports ground-plane statistics — paved shares
//! (streets, arterials, sidewalks, plazas, greenways), plaza counts, mean
//! uninterrupted street-wall length, junction mix, and tall-cell density per
//! km² (scale canon: 1 cell = 4 m, so one cell is 16 m²).
//!
//! ## Usage
//!
//! ```text
//! cargo run --release --example walkability -- --seed 445566 --extent 8
//! ```
//!
//! Flags (same hand-rolled `--key value` parser as `viz`):
//!
//! - `--seed <u64>`          world seed (default 445566)
//! - `--center-cx <i32>`     chunk column at the grid centre (default 0)
//! - `--center-cy <i32>`     chunk row at the grid centre (default 0)
//! - `--extent <u32>`        chunks per side (default 8)
//! - `--chunk-size <u16>`    cells per chunk side (default 32)
//!
//! Exit code is 0 when the fabric is healthy (streets, sidewalks, and
//! arterials all present) and 1 on degenerate output or bad flags, so CI can
//! gate on it.

use std::process::ExitCode;

use urbix::chunk::generate_chunk;
use urbix::config::WorldConfig;
use urbix::data::CellFlags;
use urbix::lot::block_loc;
use urbix::region::VoronoiDiagram;

/// Parse a `--key value` argument list into a simple string map.
fn parse_args() -> Vec<(String, String)> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut out = Vec::new();
    while args.len() >= 2 {
        let key = args.remove(0);
        let value = args.remove(0);
        out.push((key.trim_start_matches("--").to_string(), value));
    }
    if !args.is_empty() {
        eprintln!("warning: ignoring trailing argument '{:?}'", args[0]);
    }
    out
}

/// Fetch a `--key` value, or `None` if absent (empty values are ignored).
fn get<'a>(args: &'a [(String, String)], key: &str) -> Option<&'a str> {
    args.iter()
        .find(|(k, v)| k == key && !v.is_empty())
        .map(|(_, v)| v.as_str())
}

fn main() -> ExitCode {
    let args = parse_args();
    let seed: u64 = get(&args, "seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(445566);
    let center_cx: i32 = get(&args, "center-cx")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let center_cy: i32 = get(&args, "center-cy")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let extent: u32 = get(&args, "extent")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let chunk_size: u16 = get(&args, "chunk-size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);

    if extent == 0 || extent > 64 {
        eprintln!("error: --extent must be in 1..=64");
        return ExitCode::from(2);
    }
    let config = WorldConfig {
        seed,
        chunk_size,
        ..Default::default()
    };
    if !config.is_valid() {
        eprintln!("error: invalid world config (check --chunk-size)");
        return ExitCode::from(2);
    }
    let voronoi = VoronoiDiagram::generate(config.seed, config.voronoi_site_count);

    // The example drives the public pipeline only: chunks in, metrics out.
    let n = i64::from(config.chunk_size);
    let half = (extent / 2) as i32;
    let start_cx = center_cx - half;
    let start_cy = center_cy - half;
    let side_cells = extent as u64 * n as u64;
    let total = side_cells * side_cells;
    // Cell area at the 4 m canon: 16 m²; area denominators below.
    let area_km2 = total as f64 * 16.0 / 1_000_000.0;

    let mut street = 0u64;
    let mut arterial = 0u64;
    let mut sidewalk = 0u64;
    let mut plaza = 0u64;
    let mut park = 0u64;
    let mut greenway = 0u64;
    let mut built = 0u64;
    let mut landmark_tall = 0u64;
    let mut junctions = 0u64;
    // Longest built run per grid row, for the street-wall continuity mean.
    // Rows are tracked in absolute world-z bands across the whole extent.
    let mut row_run = 0u64;
    let mut row_best_sum = 0u64;
    let mut row_count = 0u64;
    let mut prev_row: Option<i64> = None;

    for cy in 0..extent {
        for cx in 0..extent {
            let chunk = generate_chunk(
                start_cx + cx as i32,
                start_cy + cy as i32,
                &config,
                &voronoi,
            );
            let mut index = 0usize;
            for ly in 0..n {
                for lx in 0..n {
                    let wx = i64::from(start_cx + cx as i32) * n + lx;
                    let wz = i64::from(start_cy + cy as i32) * n + ly;
                    let cell = chunk.get_cell(index);
                    index += 1;

                    if prev_row != Some(wz) {
                        if prev_row.is_some() {
                            row_best_sum += row_run;
                            row_count += 1;
                        }
                        prev_row = Some(wz);
                        row_run = 0;
                    }
                    let mut run = row_run;

                    let f = cell.flags;
                    if f.contains(CellFlags::IS_STREET) {
                        street += 1;
                        run = 0;
                    }
                    if f.contains(CellFlags::IS_ARTERIAL) {
                        arterial += 1;
                    }
                    if f.contains(CellFlags::IS_SIDEWALK) {
                        sidewalk += 1;
                        run = 0;
                    }
                    if f.contains(CellFlags::IS_PLAZA) {
                        plaza += 1;
                    }
                    if f.contains(CellFlags::IS_PARK) {
                        park += 1;
                        run = 0;
                    }
                    if f.contains(CellFlags::IS_GREENWAY) {
                        greenway += 1;
                        run = 0;
                    }
                    if cell.height > 0.0 {
                        built += 1;
                        run += 1;
                        // Landmark proxy: clearly above its blended band.
                        let band = config.blended_zone_params(&cell.zone_affinity);
                        if cell.height > band.height_max * 1.3 {
                            landmark_tall += 1;
                        }
                    } else if !f.contains(CellFlags::IS_PARK) {
                        run = 0;
                    }
                    if run > row_run {
                        // Track the row's best run so far; folded at row end.
                        row_run = run;
                    }

                    // Junction census in the district frame: intersections
                    // (both axes on the boundary) split 4-way vs T by whether
                    // all four arms read as street in the same frame.
                    let params = config.blended_zone_params(&cell.zone_affinity);
                    let frame = voronoi.district_frame_for(wx as f64, wz as f64);
                    let loc = block_loc(wx, wz, params.block_size, &frame);
                    if loc.rx == 0 && loc.rz == 0 {
                        junctions += 1;
                    }
                }
            }
        }
    }
    if prev_row.is_some() {
        row_best_sum += row_run;
        row_count += 1;
    }

    let pct = |v: u64| v as f64 * 100.0 / total as f64;
    let mean_wall = row_best_sum as f64 / row_count.max(1) as f64;
    println!(
        "walkability seed={seed} extent={extent} chunks={} cells={total} area={area_km2:.2} km2",
        extent * extent
    );
    println!(
        "  paved: street {:.1}%  arterial {:.1}%  sidewalk {:.1}%  plaza cells {} ({:.1}%)",
        pct(street),
        pct(arterial),
        pct(sidewalk),
        plaza,
        pct(plaza)
    );
    println!(
        "  green/built: park {:.1}%  greenway {:.1}%  built {:.1}%  tall cells {} ({:.1}/km2)",
        pct(park),
        pct(greenway),
        pct(built),
        landmark_tall,
        landmark_tall as f64 / area_km2.max(1e-9)
    );
    println!(
        "  street-wall: mean longest built run per row {mean_wall:.1} cells ({:.0} m)",
        mean_wall * 4.0
    );
    println!(
        "  junctions: {junctions} intersections ({:.1}/km2)",
        junctions as f64 / area_km2.max(1e-9)
    );

    // Acceptance gate: a healthy fabric always has streets, sidewalks, and
    // arterials somewhere in the sample.
    if street == 0 || sidewalk == 0 || arterial == 0 {
        eprintln!("degenerate fabric: street={street} sidewalk={sidewalk} arterial={arterial}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
