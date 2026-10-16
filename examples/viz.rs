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
//!   so the skyline reads instantly). Paved cells draw the ground-plane
//!   hierarchy instead (arterial / street / sidewalk / plaza). Flow avenues
//!   (M16 desire paths) draw teal so grown boulevards read apart from the
//!   lattice grid.
//! - **Affinity mode** (`--mode affinity`): shows the *dominant* zone per
//!   cell as a flat district map, ignoring height.
//! - **Walk mode** (`--mode walk`): the pedestrian's map — paved hierarchy in
//!   high-contrast tones, parks vivid green, buildings as dark masses so the
//!   walkable ground reads first (flow avenues stay teal).
//! - **Massing mode** (`--mode massing`): the split-grammar readout (M17) —
//!   podium edges amber, tower cores in their brightened zone hue, crowned
//!   cores gold; unstepped rules fall back to hybrid. Paved cells keep the
//!   hierarchy colours for orientation.
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
//! - `--mode <hybrid|affinity|walk|massing>` colouring (default hybrid)
//! - `--out <path>`          output base path (default `out`)
//! - `--inspect <wx,wz>`     print the interior report for the cell at absolute
//!   world coordinates `(wx,wz)` after rendering (see `examples/cli_demo.rs`)

use std::collections::BTreeMap;
use std::process::ExitCode;

use urbix::chunk::generate_chunk;
use urbix::config::WorldConfig;
use urbix::data::{Cell, CellFlags, ZONE_COUNT};
use urbix::interior::generate_layout_with_ground;
use urbix::layout::{Floor, Tile};
use urbix::lot::{block_loc, lot_offset, lot_rect, lot_slot};
use urbix::massing::{is_flat, lot_crowns, massing_ring};
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
/// Arterial avenue colour (`IS_ARTERIAL`): wide roads read brighter.
const ARTERIAL_RGB: [u8; 3] = [86, 72, 110];
/// Sidewalk apron colour (`IS_SIDEWALK`): pale pavement, not asphalt.
const SIDEWALK_RGB: [u8; 3] = [118, 118, 128];
/// Plaza colour (`IS_PLAZA`): warm paved square.
const PLAZA_RGB: [u8; 3] = [216, 200, 160];
/// Walk-mode park green: vivid so green masses pop against pavement.
const WALK_PARK_RGB: [u8; 3] = [64, 170, 80];
/// Walk-mode building mass: dark silhouette so the ground plane leads.
const WALK_BUILDING_RGB: [u8; 3] = [28, 32, 44];
/// Flow-avenue tint (Milestone 16): grown boulevards read apart from the
/// lattice arterials in hybrid/walk modes.
const FLOW_RGB: [u8; 3] = [52, 186, 174];
/// Massing-mode podium edge (Milestone 17): street-wall ring cells.
const PODIUM_RGB: [u8; 3] = [206, 134, 44];
/// Massing-mode crowned core: lots whose crown roll hit, inner cells only.
const CROWN_RGB: [u8; 3] = [255, 210, 90];

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

/// Compute the blended `ZoneParams` for a cell from its affinity vector,
/// through the config (heights/density blend; block size snaps from the
/// dominant zone — the same resolution generation uses).
fn zone_params_of(cell: &Cell, config: &WorldConfig) -> urbix::zones::ZoneParams {
    config.blended_zone_params(&cell.zone_affinity)
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
/// Paved cells always draw the ground-plane hierarchy (plaza, arterial,
/// sidewalk, street) and skip building/grading logic. In affinity mode the
/// flat dominant-zone hue is returned. Walk mode mutes buildings to dark
/// masses and pops parks so the pedestrian network reads first; hybrid mode
/// brightens the zone hue by height. Massing mode is resolved by the caller
/// (see `massing_role_colour`); this function covers the shared ground.
fn colour_cell(cell: &Cell, mode: &str, config: &WorldConfig) -> [u8; 3] {
    // Paved hierarchy first (plaza keeps IS_STREET, so test it first).
    if cell.flags.contains(CellFlags::IS_PLAZA) {
        return PLAZA_RGB;
    }
    if cell.flags.contains(CellFlags::IS_ARTERIAL) {
        return ARTERIAL_RGB;
    }
    if cell.flags.contains(CellFlags::IS_SIDEWALK) {
        return SIDEWALK_RGB;
    }
    // Greenways read as park green in every mode (vivid in walk mode).
    if cell.flags.contains(CellFlags::IS_GREENWAY) {
        return if mode == "walk" {
            WALK_PARK_RGB
        } else {
            ZONE_HUES[4]
        };
    }
    if cell.flags.contains(CellFlags::IS_STREET) {
        return ROAD_RGB;
    }

    let zone = dominant_zone(cell);
    // Prefer the config's hues (modular customization); fallback to compiled-in.
    let hue = config
        .zone_hues
        .get(zone)
        .copied()
        .unwrap_or(ZONE_HUES[zone]);

    if mode == "affinity" {
        return hue;
    }
    if mode == "walk" {
        if cell.flags.contains(CellFlags::IS_PARK) {
            return WALK_PARK_RGB;
        }
        if cell.height <= 0.0 {
            return hue;
        }
        return WALK_BUILDING_RGB;
    }

    // Hybrid: brighten the zone hue by height so the skyline reads. Normalize
    // against the blended zone max height so each district's range fills the
    // full brightness band. Guard the degenerate max==0 case.
    hybrid_brighten(cell, hue, config)
}

/// Brighten a zone hue toward white by building height (hybrid core).
fn hybrid_brighten(cell: &Cell, hue: [u8; 3], config: &WorldConfig) -> [u8; 3] {
    let params = zone_params_of(cell, config);
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

/// Massing-mode colour for a built cell (Milestone 17): podium edge, tower
/// core, or crowned core — recomputed through the same public helpers
/// generation uses, so the map shows exactly what the steps did. Unstepped
/// rules (flat tables) fall back to the hybrid brightening.
fn massing_role_colour(
    cell: &Cell,
    config: &WorldConfig,
    voronoi: &VoronoiDiagram,
    world_x: i64,
    world_z: i64,
) -> [u8; 3] {
    let zone = dominant_zone(cell);
    let mp = &config.massing[zone];
    let hue = config
        .zone_hues
        .get(zone)
        .copied()
        .unwrap_or(ZONE_HUES[zone]);
    if is_flat(mp) {
        return hybrid_brighten(cell, hue, config);
    }
    let params = config.blended_zone_params(&cell.zone_affinity);
    let frame = voronoi.district_frame_for(world_x as f64, world_z as f64);
    let block = block_loc(world_x, world_z, params.block_size, &frame);
    let rect = lot_rect(&block, params.block_size, config.seed);
    let off = lot_offset(&block, params.block_size, config.seed);
    let ring = massing_ring(rect, off);
    if ring < mp.tower_inset {
        return PODIUM_RGB;
    }
    let slot = lot_slot(&block, params.block_size, config.seed);
    if lot_crowns(slot.lot_id, mp, config.seed) {
        return CROWN_RGB;
    }
    hybrid_brighten(cell, hue, config)
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
                    let wx = i64::from(start_cx + cx as i32) * i64::from(n) + i64::from(x);
                    let wz = i64::from(start_cy + cy as i32) * i64::from(n) + i64::from(y);
                    let px = cx * u32::from(config.chunk_size) + x as u32;
                    let py = cy * u32::from(config.chunk_size) + y as u32;
                    let dst = ((py * grid_w + px) * 3) as usize;
                    let mut rgb = if mode == "massing" && cell.height > 0.0 {
                        massing_role_colour(&cell, config, voronoi, wx, wz)
                    } else {
                        colour_cell(&cell, mode, config)
                    };
                    // Flow avenues (Milestone 16) read apart from the lattice
                    // grid in hybrid/walk: grown boulevards draw teal. The
                    // query is world-space and chunk-consistent, like the
                    // pipeline's own check.
                    if (mode == "hybrid" || mode == "walk")
                        && cell.flags.contains(CellFlags::IS_ARTERIAL)
                        && voronoi.flow_arterial_at(wx as f64, wz as f64, config.flow_half_width)
                    {
                        rgb = FLOW_RGB;
                    }
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

/// Legend glyph for a tile (shared with `examples/cli_demo.rs`; furnished
/// rooms print uppercase).
fn tile_glyph(tile: Tile, kind: u8, furn: u8) -> char {
    match tile {
        Tile::Void => '.',
        Tile::Wall => '#',
        Tile::Door => 'D',
        Tile::Core => '+',
        Tile::Corridor => ' ',
        Tile::Room => {
            let c = char::from(b'a' + kind % 26);
            if furn == 0 {
                c
            } else {
                c.to_ascii_uppercase()
            }
        }
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
                row.push(tile_glyph(floor.tiles[i], floor.kinds[i], floor.furn[i]));
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
pub fn interior_report(
    config: &WorldConfig,
    voronoi: &urbix::region::VoronoiDiagram,
    world_x: i64,
    world_z: i64,
    cell: &Cell,
) -> String {
    let mut out = String::new();

    let kind = if cell.flags.contains(CellFlags::IS_PLAZA) {
        "plaza"
    } else if cell.flags.contains(CellFlags::IS_GREENWAY) {
        "greenway"
    } else if cell.flags.contains(CellFlags::IS_ARTERIAL) {
        "arterial street"
    } else if cell.flags.contains(CellFlags::IS_SIDEWALK) {
        "sidewalk"
    } else if cell.flags.contains(CellFlags::IS_STREET) {
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

    let ctx = urbix::chunk::interior_context_for(config, voronoi, world_x, world_z, cell);
    out.push_str(&format!(
        "context: zone {:?} (secondary {:?}, role {:?}{}), footprint {}x{} tiles, {} floors, entrance {:?}, frontage depth {}\n",
        ctx.zone,
        ctx.secondary_zone,
        ctx.building_role,
        if ctx.corner { ", corner" } else { "" },
        ctx.footprint_w,
        ctx.footprint_d,
        ctx.floor_count,
        ctx.door_side,
        ctx.frontage_depth
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

    // Massing readout (Milestone 17): the split-grammar rule for the cell's
    // dominant zone plus this cell's role in it (podium edge / tower core /
    // crowned), recomputed through the same public helpers generation uses.
    {
        use urbix::lot::{block_loc, lot_offset, lot_rect, lot_slot};
        use urbix::massing::{lot_crowns, massing_ring, podium_top};
        let params = config.blended_zone_params(&cell.zone_affinity);
        let frame = voronoi.district_frame_for(world_x as f64, world_z as f64);
        let block = block_loc(world_x, world_z, params.block_size, &frame);
        let slot = lot_slot(&block, params.block_size, config.seed);
        let rect = lot_rect(&block, params.block_size, config.seed);
        let off = lot_offset(&block, params.block_size, config.seed);
        let mp = &config.massing[ctx.zone as usize];
        let ring = massing_ring(rect, off);
        let roof = ["flat", "pitched", "sawtooth"]
            .get(usize::from(mp.roof_mode))
            .copied()
            .unwrap_or("unknown");
        let role = if ring < mp.tower_inset {
            "podium edge"
        } else if lot_crowns(slot.lot_id, mp, config.seed) {
            "crowned core"
        } else {
            "tower core"
        };
        out.push_str(&format!(
            "massing: {:?} podium {} floors/inset {} {} roof crown {:.0}% — cell ring {ring} (podium top {:.1} m), {role}\n",
            ctx.zone,
            mp.podium_floors,
            mp.tower_inset,
            roof,
            mp.crown_share * 100.0,
            podium_top(mp, config.interior_floor_height)
        ));
    }

    let layout = {
        let ground =
            urbix::interior::resolve_ground_override(&ctx, &bp, &config.interior_blueprints);
        generate_layout_with_ground(cell.interior_id, &ctx, &bp, ground)
    };
    {
        let gw = usize::from(ctx.footprint_w.max(1));
        let shafts =
            urbix::interior::building_shafts(cell.interior_id, ctx.seed, gw, bp.wet_shafts);
        let ground_bp =
            urbix::interior::resolve_ground_override(&ctx, &bp, &config.interior_blueprints);
        let units =
            urbix::interior::unit_rects_for_floor(cell.interior_id, &ctx, 0, ground_bp, &bp);
        let furn: usize = layout
            .floors
            .iter()
            .map(|f| f.furn.iter().filter(|c| **c != 0).count())
            .sum();
        out.push_str(&format!(
            "structure: {} ground unit(s), shafts {:?}, {} furnished tiles\n",
            units.len(),
            shafts,
            furn
        ));
    }
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
    out.push_str("legend: # wall, + core, D door, ' ' corridor, a..z rooms (UPPERCASE = furnished), . void\n");
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
        Some("walk") => "walk",
        Some("massing") => "massing",
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
        print!("{}", interior_report(&config, &voronoi, wx, wz, &cell));
    }

    ExitCode::SUCCESS
}

/// Write a PNG via the `image` dev-dependency (never part of the library).
fn write_png(path: &str, width: u32, height: u32, pixels: &[u8]) -> image::ImageResult<()> {
    let img: image::RgbImage = image::ImageBuffer::from_raw(width, height, pixels.to_vec())
        .expect("buffer sized correctly");
    img.save(path)
}
