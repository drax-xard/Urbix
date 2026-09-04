//! # viz.rs — 2D city visualizer
//!
//! A lightweight, dependency-light example that renders the engine's chunk
//! generation to images so you can eyeball what the system actually produces.
//! It is *not* part of the library: it drives the public generation pipeline
//! (`generate_chunk`) directly and writes the result as an image.
//!
//! ## What it draws
//!
//! A grid of `extent × extent` chunks is generated for a seed. Every cell is
//! mapped to an RGB pixel, top-left chunk first, one pixel per cell:
//!
//! - **Hybrid mode (default)**: each cell is tinted by its district's zone
//!   colour, then brightened toward white by building height (tall = bright,
//!   so the skyline reads instantly). Street cells (flag `IS_STREET`) are
//!   painted as roads instead.
//! - **Affinity mode** (`--mode affinity`): shows the *dominant* zone per
//!   cell as a flat district map, ignoring height.
//!
//! Two files are always written:
//! - `.ppm` — P6 binary PPM, written by hand (zero dependencies).
//! - `.png` — via the `image` dev-dependency.
//!
//! ## Usage
//!
//! ```text
//! cargo run --example viz -- --seed 445566 --extent 16
//! # ...also dump the interior of the lot at world (120, 400):
//! cargo run --example viz -- --seed 445566 --inspect 120,400
//! ```
//!
//! Flags (simple positional-key parser, hand-rolled until Milestone 7 wires up
//! `clap`):
//!
//! - `--seed <u64>`          world seed (default 0)
//! - `--center-cx <i32>`     chunk column at the grid centre (default 0)
//! - `--center-cy <i32>`     chunk row at the grid centre (default 0)
//! - `--extent <u32>`        chunks per side (default 16; 16 → 512×512 px)
//! - `--chunk-size <u16>`    cells per chunk side (default 32)
//! - `--mode <hybrid|affinity>` colouring (default hybrid)
//! - `--out <path>`          output base path (default `out`)
//! - `--inspect <wx,wz>`     print the interior report for the cell at absolute
//!   world coordinates `(wx,wz)` after rendering (see `examples/cli_demo.rs`)

use std::collections::BTreeMap;
use std::process::ExitCode;

use urbix::chunk::generate_chunk;
use urbix::config::WorldConfig;
use urbix::data::{Cell, CellFlags, ZONE_COUNT};
use urbix::interior::generate_layout;
use urbix::layout::{Floor, Tile};
use urbix::region::VoronoiDiagram;

/// Base hue (RGB before height brightening) for each district.
///
/// Promoted to `WorldConfig::zone_hues` in Milestone 8 so artists tune without
/// recompiling; this fallback matches `WorldConfig::default().zone_hues`.
const ZONE_HUES: [[u8; 3]; ZONE_COUNT] = [
    [100, 150, 220], // Downtown    — steel blue
    [96, 180, 90],   // Residential — tree green
    [235, 160, 70],  // Commercial  — warm orange
    [150, 130, 115], // Industrial  — grimy grey/brown
    [140, 205, 120], // Park        — light green
];

/// Road colour for `IS_STREET` cells.
const ROAD_RGB: [u8; 3] = [40, 40, 46];

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

/// Compute the blended `ZoneParams` for a cell from its affinity vector.
fn zone_params_of(cell: &Cell) -> urbix::zones::ZoneParams {
    urbix::zones::zone_params(&cell.zone_affinity)
}

/// The dominant zone index for a cell (argmax over affinity).
fn dominant_zone(cell: &Cell) -> usize {
    let mut best = 0usize;
    let mut best_w = -1.0f32;
    for (i, w) in cell.zone_affinity.iter().enumerate() {
        if *w > best_w {
            best = i;
            best_w = *w;
        }
    }
    best
}

/// Map a cell to an RGB pixel.
///
/// In hybrid mode the returned colour is the zone hue brightened by the
/// building height (relative to the blended zone max); streets draw the road
/// colour. In affinity mode the flat dominant-zone hue is returned.
fn colour_cell(cell: &Cell, mode: &str) -> [u8; 3] {
    // Streets are always drawn as roads and skip building/grading logic.
    if cell.flags.contains(CellFlags::IS_STREET) {
        return ROAD_RGB;
    }

    let zone = dominant_zone(cell);
    let hue = ZONE_HUES[zone];

    if mode == "affinity" {
        return hue;
    }

    // Hybrid: brighten the zone hue by height so the skyline reads. Normalize
    // against the blended zone max height so each district's range fills the
    // full brightness band. Guard the degenerate max==0 case.
    let params = zone_params_of(cell);
    let scale = if params.height_max > f32::EPSILON {
        (cell.height / params.height_max).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let to_u8 = |c: u8, t: f32| (f32::from(c) * (1.0 - t) + 255.0 * t) as u8;
    [
        to_u8(hue[0], scale),
        to_u8(hue[1], scale),
        to_u8(hue[2], scale),
    ]
}

/// Render a grid of chunks into a flat RGB pixel buffer (one pixel per cell).
fn render(
    config: &WorldConfig,
    voronoi: &VoronoiDiagram,
    center_cx: i32,
    center_cy: i32,
    extent: u32,
    mode: &str,
) -> Vec<u8> {
    let n = i32::from(config.chunk_size);
    let half = (extent / 2) as i32;
    // Even extents centre the grid on the requested chunk; odd extents sit
    // between chunks. To keep it simple, we anchor the top-left so the number
    // of chunks is exactly `extent` on each side.
    let start_cx = center_cx - half;
    let start_cy = center_cy - half;

    let grid_w = extent * u32::from(config.chunk_size);
    let grid_h = extent * u32::from(config.chunk_size);
    let mut pixels = vec![0u8; (grid_w * grid_h * 3) as usize];

    for cy in 0..extent {
        for cx in 0..extent {
            let chunk = generate_chunk(start_cx + cx as i32, start_cy + cy as i32, config, voronoi);
            let mut index = 0usize;
            for y in 0..n {
                for x in 0..n {
                    let cell = chunk.get_cell(index);
                    index += 1;
                    let px = cx * u32::from(config.chunk_size) + x as u32;
                    let py = cy * u32::from(config.chunk_size) + y as u32;
                    let dst = ((py * grid_w + px) * 3) as usize;
                    let rgb = colour_cell(&cell, mode);
                    pixels[dst] = rgb[0];
                    pixels[dst + 1] = rgb[1];
                    pixels[dst + 2] = rgb[2];
                }
            }
        }
    }
    pixels
}

/// Write a P6 binary PPM image: header then raw RGB triplets.
fn write_ppm(path: &str, width: u32, height: u32, pixels: &[u8]) -> std::io::Result<()> {
    let mut data = Vec::with_capacity(15 + pixels.len());
    data.extend_from_slice(format!("P6\n{width} {height}\n255\n").as_bytes());
    data.extend_from_slice(pixels);
    std::fs::write(path, data)
}

/// Legend glyph for a tile (shared with `examples/cli_demo.rs`).
fn tile_glyph(tile: Tile, kind: u8) -> char {
    match tile {
        Tile::Void => '.',
        Tile::Wall => '#',
        Tile::Door => 'D',
        Tile::Core => '+',
        Tile::Corridor => ' ',
        Tile::Room => char::from(b'a' + kind % 26),
    }
}

/// One floor rendered as rows of legend glyphs.
fn floor_rows(floor: &Floor) -> Vec<String> {
    let w = usize::from(floor.width);
    (0..usize::from(floor.depth))
        .map(|z| {
            let mut row = String::with_capacity(w);
            for x in 0..w {
                let i = z * w + x;
                row.push(tile_glyph(floor.tiles[i], floor.kinds[i]));
            }
            row
        })
        .collect()
}

/// Count `Room` tiles per opaque room-kind tag on one floor.
fn room_tile_counts(floor: &Floor) -> BTreeMap<u8, usize> {
    let mut counts = BTreeMap::new();
    for i in 0..floor.tiles.len() {
        if floor.tiles[i] == Tile::Room {
            *counts.entry(floor.kinds[i]).or_insert(0) += 1;
        }
    }
    counts
}

/// Full textual report on the interior of the cell at absolute world
/// coordinates `(world_x, world_z)` — same output as `examples/cli_demo.rs`.
#[must_use]
pub fn interior_report(config: &WorldConfig, world_x: i64, world_z: i64, cell: &Cell) -> String {
    let mut out = String::new();

    let kind = if cell.flags.contains(CellFlags::IS_STREET) {
        "street"
    } else if cell.flags.contains(CellFlags::IS_PARK) {
        "park"
    } else {
        "lot"
    };
    out.push_str(&format!(
        "cell ({world_x},{world_z}) [{kind}] height {} u, palette {}, interior id {:#x}\n",
        cell.height, cell.palette_id, cell.interior_id
    ));

    if cell.height <= 0.0 {
        out.push_str("no building on this cell — no interior\n");
        return out;
    }

    let ctx = urbix::chunk::interior_context_for(config, world_x, world_z, cell);
    out.push_str(&format!(
        "context: zone {:?}, footprint {}x{} tiles, {} floors, entrance {:?}\n",
        ctx.zone, ctx.footprint_w, ctx.footprint_d, ctx.floor_count, ctx.door_side
    ));

    let bp = config.blueprint_for(ctx.zone);
    out.push_str(&format!(
        "blueprint: wall margin {}, core {} tiles, {} room template(s)\n",
        bp.margin,
        bp.core_size,
        bp.room_slice().len()
    ));
    for room in bp.room_slice() {
        out.push_str(&format!(
            "  kind {:>2}: weight {:>4.1}, size {}..{} x {}..{}\n",
            room.kind, room.weight, room.min_w, room.max_w, room.min_d, room.max_d
        ));
    }

    let layout = generate_layout(cell.interior_id, &ctx, &bp);
    out.push_str(&format!(
        "{} storey(s), seed {}\n",
        layout.floors.len(),
        layout.seed
    ));
    for (f, floor) in layout.floors.iter().enumerate() {
        out.push_str(&format!(
            "-- floor {f} ({}x{} tiles)\n",
            floor.width, floor.depth
        ));
        let mut walls = 0;
        let mut doors = 0;
        let mut cores = 0;
        let mut corridors = 0;
        let mut rooms = 0;
        for tile in &floor.tiles {
            match tile {
                Tile::Wall => walls += 1,
                Tile::Door => doors += 1,
                Tile::Core => cores += 1,
                Tile::Corridor => corridors += 1,
                Tile::Room => rooms += 1,
                Tile::Void => {}
            }
        }
        out.push_str(&format!(
            "  tiles: {rooms} rooms, {corridors} corridor, {walls} wall, {cores} core, {doors} door\n"
        ));
        let room_kinds = room_tile_counts(floor);
        if room_kinds.is_empty() {
            out.push_str("  rooms: (none)\n");
        } else {
            let summary = room_kinds
                .iter()
                .map(|(kind, n)| format!("kind {kind} x {n}"))
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!("  rooms: {summary}\n"));
        }
        for row in floor_rows(floor) {
            out.push_str("  ");
            out.push_str(&row);
            out.push('\n');
        }
    }
    out.push_str("legend: # wall, + core, D door, ' ' corridor, a..z rooms, . void\n");
    out
}

fn main() -> ExitCode {
    let args = parse_args();
    let seed = get(&args, "seed").and_then(|s| s.parse().ok()).unwrap_or(0);
    let center_cx = get(&args, "center-cx")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let center_cy = get(&args, "center-cy")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let extent = get(&args, "extent")
        .and_then(|s| s.parse().ok())
        .unwrap_or(16);
    let chunk_size = get(&args, "chunk-size")
        .and_then(|s| s.parse().ok())
        .unwrap_or(32);
    let mode = match get(&args, "mode") {
        Some("affinity") => "affinity",
        _ => "hybrid",
    };
    let out_base = get(&args, "out").unwrap_or("out").to_string();
    let inspect: Option<(i64, i64)> = get(&args, "inspect").and_then(|s| {
        let (a, b) = s.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    });

    if extent == 0 {
        eprintln!("error: --extent must be > 0");
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

    let grid_w = extent * u32::from(config.chunk_size);
    let grid_h = extent * u32::from(config.chunk_size);
    println!(
        "Generating {extent}x{extent} chunks ({}x{} px) centred on chunk ({center_cx},{center_cy}), seed {seed}",
        grid_w, grid_h
    );
    let pixels = render(&config, &voronoi, center_cx, center_cy, extent, mode);

    let ppm_path = format!("{out_base}.ppm");
    if let Err(e) = write_ppm(&ppm_path, grid_w, grid_h, &pixels) {
        eprintln!("error writing PPM: {e}");
        return ExitCode::from(1);
    }
    println!("wrote {ppm_path}");

    // PNG output uses the `image` dev-dependency.
    if let Err(e) = write_png(&format!("{out_base}.png"), grid_w, grid_h, &pixels) {
        eprintln!("error writing PNG: {e}");
        return ExitCode::from(1);
    }

    if let Some((wx, wz)) = inspect {
        let n = i64::from(config.chunk_size);
        let cx = wx.div_euclid(n);
        let cz = wz.div_euclid(n);
        let lx = wx.rem_euclid(n);
        let lz = wz.rem_euclid(n);
        let chunk = generate_chunk(cx as i32, cz as i32, &config, &voronoi);
        let cell = chunk.get_cell((lz * n + lx) as usize);
        print!("{}", interior_report(&config, wx, wz, &cell));
    }

    ExitCode::SUCCESS
}

/// Write a PNG via the `image` dev-dependency (never part of the library).
fn write_png(path: &str, width: u32, height: u32, pixels: &[u8]) -> image::ImageResult<()> {
    let img: image::RgbImage = image::ImageBuffer::from_raw(width, height, pixels.to_vec())
        .expect("buffer sized correctly");
    img.save(path)
}
