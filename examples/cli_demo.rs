//! # cli_demo.rs — inspect the interior of a selected exterior cell
//!
//! A headless, wall-of-text demo that drives the public generation pipeline
//! and reports on the interior mini-world generated for a chosen built cell:
//! the exact `InteriorContext` the exterior lot produced (zone, floors,
//! footprint, entrance side), the per-zone `Blueprint` that shaped it, and
//! every storey's tile grid.
//!
//! It ties together the three public APIs a consumer uses to read an interior
//! (`chunk::interior_context_for` → the context, `WorldConfig::blueprint_for`
//! → the rule table, `interior::generate_layout` → the mini-world), so it
//! doubles as a living example of the Milestone-9 surface.
//!
//! ## Usage
//!
//! ```text
//! cargo run --example cli_demo -- --seed 445566 --cx 0 --cy 0
//! # ...or pick a specific local cell inside chunk (0,0):
//! cargo run --example cli_demo -- --seed 445566 --cx 0 --cy 0 --dx 12 --dy 7
//! ```
//!
//! Flags (simple `--key value` parser, matching `examples/viz.rs`):
//!
//! - `--seed <u64>`  world seed (default 445566)
//! - `--cx <i32>`    chunk column (default 0)
//! - `--cy <i32>`    chunk row (default 0)
//! - `--dx <u32>`    local cell column inside the chunk; absent → tallest built cell
//! - `--dy <u32>`    local cell row inside the chunk; absent → tallest built cell
//!
//! `dx`/`dy` select one cell of the selected chunk (clamped to its extent).
//! By default the tallest building in the chunk is inspected instead.

use std::collections::BTreeMap;
use std::process::ExitCode;

use urbix::chunk::interior_context_for;
use urbix::config::WorldConfig;
use urbix::data::{Cell, CellFlags};
use urbix::engine::WorldEngine;
use urbix::interior::generate_layout;
use urbix::layout::{Floor, Tile};

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

/// Legend glyph for a tile, reusing the convention of the interior unit tests
/// (rooms appear as letters, circulation as blank space).
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

/// Tile-kind histogram for one floor: `(Wall, Door, Core, Corridor, Room)`.
fn tile_counts(floor: &Floor) -> (usize, usize, usize, usize, usize) {
    let mut counts = (0, 0, 0, 0, 0);
    for tile in &floor.tiles {
        match tile {
            Tile::Wall => counts.0 += 1,
            Tile::Door => counts.1 += 1,
            Tile::Core => counts.2 += 1,
            Tile::Corridor => counts.3 += 1,
            Tile::Room => counts.4 += 1,
            Tile::Void => {}
        }
    }
    counts
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
/// coordinates `(world_x, world_z)`.
///
/// Prints the exterior cell's summary, its reconstructed `InteriorContext` and
/// `Blueprint`, then each storey: a tile histogram, per-kind room-tile counts,
/// and an ASCII grid. Cells that cannot have an interior (streets, bare lots)
/// are reported as such.
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

    let ctx = interior_context_for(config, world_x, world_z, cell);
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
        let (walls, doors, cores, corridors, rooms) = tile_counts(floor);
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
    let seed = get(&args, "seed")
        .and_then(|s| s.parse().ok())
        .unwrap_or(445566);
    let cx = get(&args, "cx").and_then(|s| s.parse().ok()).unwrap_or(0);
    let cy = get(&args, "cy").and_then(|s| s.parse().ok()).unwrap_or(0);
    let dx: Option<u32> = get(&args, "dx").and_then(|s| s.parse().ok());
    let dy: Option<u32> = get(&args, "dy").and_then(|s| s.parse().ok());

    let cfg = WorldConfig {
        seed,
        ..Default::default()
    };
    if !cfg.is_valid() {
        eprintln!("error: invalid world config");
        return ExitCode::from(2);
    }

    let mut engine = WorldEngine::with_config(cfg);
    let chunk = engine.generate_chunk(cx, cy);
    let n = i64::from(cfg.chunk_size);

    // Resolve the selected cell: explicit local (dx, dy), else the tallest
    // building in the chunk.
    let (lx, ly) = match (dx, dy) {
        (Some(x), Some(y)) => {
            let lx = i64::from(x.min(u32::from(cfg.chunk_size - 1)));
            let ly = i64::from(y.min(u32::from(cfg.chunk_size - 1)));
            (lx, ly)
        }
        _ => {
            let mut best = None;
            let mut best_h = 0.0f32;
            for i in 0..chunk.cell_count() {
                let cell = chunk.get_cell(i);
                if cell.height > best_h {
                    best_h = cell.height;
                    best = Some(i as i64);
                }
            }
            match best {
                Some(i) => (i % n, i / n),
                None => {
                    eprintln!("chunk ({cx},{cy}) contains no built cells");
                    return ExitCode::from(0);
                }
            }
        }
    };

    let world_x = i64::from(cx) * n + lx;
    let world_z = i64::from(cy) * n + ly;
    let cell = chunk.get_cell((ly * n + lx) as usize);
    print!("{}", interior_report(&cfg, world_x, world_z, &cell));
    ExitCode::SUCCESS
}
