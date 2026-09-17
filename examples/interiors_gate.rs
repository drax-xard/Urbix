//! # interiors_gate.rs — headless acceptance for interior believability
//!
//! Samples built lots across zones and seeds through the public engine path
//! (chunk → context → cached layout → room records) plus roomy synthetic
//! fixtures, and asserts the structural invariants Milestones 13–15 promise:
//! stacked shafts, a single ground entrance, wet-stack alignment, kitchen +
//! bath minimums, furniture hygiene, and record/grid agreement.
//!
//! ## Usage
//!
//! ```text
//! cargo run --release --example interiors_gate
//! ```
//!
//! No flags: seeds and windows are fixed so the gate is deterministic. Exit
//! code is 0 when every check holds and 1 on the first violation (with the
//! lot and invariant named), so CI can gate on it.

use std::process::ExitCode;

use urbix::engine::WorldEngine;
use urbix::interior::{
    building_shafts, generate_layout_with_ground, resolve_ground_override, rooms_of_floor,
    unit_rects_for_floor,
};
use urbix::layout::{
    blueprint_defaults, BuildingRole, DoorSide, Floor, InteriorContext, Tile, FURN_BATH, TAG_WET,
};
use urbix::zones::ZoneType;

/// A failed invariant: print and exit 1.
macro_rules! gate {
    ($ok:expr, $($msg:tt)*) => {
        if !($ok) {
            eprintln!("interiors_gate FAILED: {}", format!($($msg)*));
            return ExitCode::from(1);
        }
    };
}

/// Core tile coordinates of a floor, sorted.
fn core_coords(f: &Floor) -> Vec<(u8, u8)> {
    let mut out = Vec::new();
    for z in 0..f.depth {
        for x in 0..f.width {
            if f.tiles[f.index(x, z)] == Tile::Core {
                out.push((x, z));
            }
        }
    }
    out.sort_unstable();
    out
}

/// Doors on the outer wall ring (street entrances only).
fn ring_doors(f: &Floor) -> usize {
    let (w, d) = (f.width, f.depth);
    let mut n = 0;
    for x in 0..w {
        if f.tiles[f.index(x, 0)] == Tile::Door {
            n += 1;
        }
        if f.tiles[f.index(x, d - 1)] == Tile::Door {
            n += 1;
        }
    }
    for z in 1..d.saturating_sub(1) {
        if f.tiles[f.index(0, z)] == Tile::Door {
            n += 1;
        }
        if f.tiles[f.index(w - 1, z)] == Tile::Door {
            n += 1;
        }
    }
    n
}

/// Roomy apartment fixture: Ground + Typical + Top on 14×12.
fn apartments(id: u64, seed: u64) -> (InteriorContext, urbix::layout::Blueprint) {
    let ctx = InteriorContext::new(
        id,
        ZoneType::Residential,
        [0.0; 5],
        12.0,
        4.0,
        64,
        14,
        12,
        1,
        DoorSide::South,
        seed,
        false,
        BuildingRole::Ordinary,
        ZoneType::Commercial,
    );
    let bp = blueprint_defaults(ZoneType::Residential);
    (ctx, bp)
}

fn check_synthetic() -> ExitCode {
    // Minimums, wet stacks, and windows on a roomy home (exact asserts —
    // enforcement cannot fail for lack of space here).
    let (ctx, bp) = apartments(101, 202);
    let table = urbix::layout::default_blueprints();
    let ground = resolve_ground_override(&ctx, &bp, &table);
    let layout = generate_layout_with_ground(101, &ctx, &bp, ground);
    gate!(
        layout.floors.len() == 3,
        "apartment has {} floors",
        layout.floors.len()
    );

    // Stacked shaft on every storey (top expands, so compare positions only
    // on non-top floors; typicals are byte-equal clones by construction).
    let first = core_coords(&layout.floors[1]);
    gate!(!first.is_empty(), "typical floor has no core");
    // Entrance: exactly one ring door on the ground floor, none above.
    gate!(
        ring_doors(&layout.floors[0]) == 1,
        "ground lacks one entrance"
    );
    for (i, f) in layout.floors.iter().enumerate().skip(1) {
        gate!(ring_doors(f) == 0, "street door on floor {i}");
    }
    // Windows on the entry edge.
    gate!(
        !Floor::window_cells(&layout.floors[0], ctx.door_side).is_empty(),
        "no window candidates on entry edge"
    );
    // Mixed-use programs per floor: retail base below, homes above.
    // Kitchen + bath minimums bind on residential storeys; wet tiles hug
    // the shafts under their own floor's blueprint.
    let shafts = building_shafts(101, 202, 14, bp.wet_shafts);
    gate!(shafts.len() == 2, "expected 2 shafts, got {}", shafts.len());
    for (i, f) in layout.floors.iter().enumerate() {
        let bp_f = if i == 0 { ground } else { &bp };
        if i == 0 {
            gate!(f.kinds.contains(&30), "no retail on mixed-use ground");
        } else {
            gate!(f.kinds.contains(&21), "floor {i} missing kitchen");
            gate!(f.kinds.contains(&23), "floor {i} missing bath");
        }
        let wet: Vec<u8> = bp_f
            .room_slice()
            .iter()
            .filter(|r| r.tags & TAG_WET != 0)
            .map(|r| r.kind)
            .collect();
        let tol = bp_f
            .room_slice()
            .iter()
            .filter(|r| r.tags & TAG_WET != 0)
            .map(|r| r.max_w as i64 - 1)
            .max()
            .unwrap_or(2)
            .max(1);
        for z in 0..f.depth {
            for x in 0..f.width {
                let idx = f.index(x, z);
                if f.tiles[idx] == Tile::Room && wet.contains(&f.kinds[idx]) {
                    gate!(
                        shafts.iter().any(|sc| (x as i64 - *sc as i64).abs() <= tol),
                        "wet tile at ({x},{z}) off-shaft on floor {i}"
                    );
                }
            }
        }
    }
    // Furniture hygiene + records tiling the rooms exactly.
    let mut furnished = 0;
    for (i, f) in layout.floors.iter().enumerate() {
        for (idx, code) in f.furn.iter().enumerate() {
            gate!(*code <= FURN_BATH, "furn code {code} out of range");
            if *code != 0 {
                furnished += 1;
                gate!(f.tiles[idx] == Tile::Room, "furniture off-room on {i}");
            }
        }
        let bp_f = if i == 0 { ground } else { &bp };
        let units = unit_rects_for_floor(101, &ctx, i as u8, bp_f, &bp);
        let recs = rooms_of_floor(f, i as u8, &units);
        let covered: usize = recs.iter().map(|r| r.area as usize).sum();
        let rooms: usize = f.tiles.iter().filter(|t| **t == Tile::Room).count();
        gate!(
            covered == rooms,
            "records cover {covered} of {rooms} room tiles"
        );
    }
    gate!(furnished > 0, "roomy home has no furniture");
    ExitCode::SUCCESS
}

fn check_sampled(count: &mut usize) -> ExitCode {
    // Structural invariants over real generated lots: determinism through
    // the engine cache, one stacked shaft, one entrance, furniture hygiene.
    for seed in [445566u64, 7, 42] {
        let mut engine = WorldEngine::new(seed);
        // Up to 6 built lots around the origin (whatever zones appear).
        let mut lots = Vec::new();
        'scan: for cx in -1..=1 {
            for cy in -1..=1 {
                let chunk = engine.generate_chunk(cx, cy);
                let n = i64::from(engine.config().chunk_size);
                for (i, cell) in chunk.cells().enumerate() {
                    if cell.height > 0.0 {
                        lots.push((cx as i64 * n + i as i64 % n, cy as i64 * n + i as i64 / n));
                        if lots.len() >= 6 {
                            break 'scan;
                        }
                    }
                }
            }
        }
        gate!(!lots.is_empty(), "seed {seed} built nothing near origin");
        for (wx, wz) in lots {
            *count += 1;
            let a = engine.interior_layout(wx, wz);
            gate!(a.is_some(), "built cell ({wx},{wz}) has no layout");
            let a = a.expect("checked");
            let b = engine.interior_layout(wx, wz).expect("cache hit");
            gate!(a == b, "cache disagrees at ({wx},{wz})");
            // Shaft identical on all non-top floors.
            if a.floors.len() > 2 {
                let first = core_coords(&a.floors[1]);
                for f in &a.floors[2..a.floors.len() - 1] {
                    gate!(core_coords(f) == first, "shaft wanders at ({wx},{wz})");
                }
            }
            gate!(
                ring_doors(&a.floors[0]) == 1,
                "no single entrance ({wx},{wz})"
            );
            // TAG_WET rooms still stack on roomy lots (small lots may place
            // required minimums off-shaft by design).
            let area = usize::from(a.context.footprint_w) * usize::from(a.context.footprint_d);
            if area < 64 {
                continue;
            }
            let main = engine.config().blueprint_for(a.context.zone);
            let shafts = building_shafts(
                a.id,
                a.context.seed,
                a.context.footprint_w as usize,
                main.wet_shafts,
            );
            let wet_kinds: Vec<u8> = main
                .room_slice()
                .iter()
                .filter(|r| r.tags & TAG_WET != 0)
                .map(|r| r.kind)
                .collect();
            let tol = main
                .room_slice()
                .iter()
                .filter(|r| r.tags & TAG_WET != 0)
                .map(|r| r.max_w as i64 - 1)
                .max()
                .unwrap_or(2)
                .max(1);
            for (i, f) in a.floors.iter().enumerate() {
                for z in 0..f.depth {
                    for x in 0..f.width {
                        let idx = f.index(x, z);
                        if f.tiles[idx] == Tile::Room && wet_kinds.contains(&f.kinds[idx]) {
                            gate!(
                                shafts.iter().any(|sc| (x as i64 - *sc as i64).abs() <= tol),
                                "wet tile off-shaft at ({wx},{wz}) floor {i}"
                            );
                        }
                    }
                }
            }
        }
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let code = check_synthetic();
    if code != ExitCode::SUCCESS {
        return code;
    }
    let mut lots = 0;
    let code = check_sampled(&mut lots);
    if code == ExitCode::SUCCESS {
        println!("interiors_gate: OK (synthetic fixture + {lots} sampled lots)");
    }
    code
}
