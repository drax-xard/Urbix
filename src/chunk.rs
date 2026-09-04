//! # chunk.rs
//!
//! Per-chunk generation orchestration for the Urbix engine.
//!
//! The world is divided into fixed-size chunks (e.g. 32×32 cells) addressed by
//! integer `(cx, cy)` coordinates. This module generates the full contents of
//! one chunk deterministically, taking a coordinate and the world state and
//! returning a flat array of cells.
//!
//! ## Pipeline (per cell)
//!
//! 1. Query the continuous Voronoi zone field (`region.rs`) at the cell's
//!    world position to obtain a blended zone-affinity vector.
//! 2. Resolve that into concrete per-zone parameters (`zones.rs`).
//! 3. Ask `street.rs` whether this cell is part of the street grid; streets
//!    carry `height = 0`.
//! 4. For non-street cells, ask `building.rs` for height, palette id, and
//!    compute the cell's `InteriorId`.
//! 5. Package the result into the `repr(C)` cell record (`data.rs`).
//!
//! ## Determinism & edge consistency
//!
//! Each chunk is generated with no cross-chunk write dependency. Adjacent
//! chunks agree at their shared edges because every cell queries the same
//! continuous Voronoi field (rather than a per-chunk local state) and every
//! derived value is keyed on the cell's **absolute** world coordinates.

use crate::building;
use crate::config::WorldConfig;
use crate::data::{Cell, CellFlags, ChunkBuffer, ChunkId, InteriorId};
use crate::hash::{domain, hash_coords};
use crate::layout::DoorSide;
use crate::region::VoronoiDiagram;
use crate::street;
use crate::zones::ZoneType;

/// Generate the full contents of one chunk deterministically.
///
/// Walks every cell in the `(cx, cy)` chunk, queries the Voronoi zone field at
/// the cell's absolute world position, and composes the street grid (`street`)
/// with building placement (`building`) to fill a [`ChunkBuffer`]. The chunk
/// is addressed by `config.seed` for its segment of the world; `voronoi` is a
/// pre-generated field shared across all chunks so their edges line up.
///
/// ## Example
///
/// ```
/// use urbix::chunk::generate_chunk;
/// use urbix::config::WorldConfig;
/// use urbix::data::ChunkId;
/// use urbix::region::VoronoiDiagram;
///
/// let cfg = WorldConfig { seed: 445566, ..Default::default() };
/// let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
/// let chunk = generate_chunk(0, 0, &cfg, &voronoi);
/// assert_eq!(chunk.header().cell_count, 32 * 32);
/// ```
#[must_use]
pub fn generate_chunk(
    cx: i32,
    cy: i32,
    config: &WorldConfig,
    voronoi: &VoronoiDiagram,
) -> ChunkBuffer {
    let chunk_size = i64::from(config.chunk_size);
    let seed = config.seed;
    let mut buf = ChunkBuffer::new(ChunkId::new(cx, cy), config.chunk_size, seed);

    let mut index = 0usize;
    for local_y in 0..chunk_size {
        for local_x in 0..chunk_size {
            // Absolute world cell coordinates: stable across chunks, so the
            // generated cell at a given world position never changes.
            // Computed in i64 so large chunk indices cannot overflow (the city
            // is infinite; cx*chunk_size + local would overflow i32).
            let world_x = i64::from(cx) * chunk_size + local_x;
            let world_z = i64::from(cy) * chunk_size + local_y;

            let affinity = voronoi.query(world_x as f64, world_z as f64);
            let params = config.blended_zone_params(&affinity);

            // Streets first; a street cell never becomes a building.
            let mut flags = street::layout_block(world_x, world_z, &params);

            // A cell dominated by the Park district (and not a street) is
            // flagged as greenery, giving the public `IS_PARK` wire flag real
            // meaning instead of remaining a never-set constant.
            if !flags.contains(CellFlags::IS_STREET)
                && dominant_zone(&affinity) == crate::zones::ZoneType::Park
            {
                flags = flags.insert(CellFlags::IS_PARK);
            }

            let mut cell = Cell {
                height: 0.0,
                zone_affinity: affinity,
                palette_id: 0,
                flags,
                _pad: 0,
                interior_id: 0,
            };

            if !flags.contains(CellFlags::IS_STREET) {
                let (height, palette) = building::assign_building(world_x, world_z, &params, seed);
                cell.height = height;
                cell.palette_id = palette;
                if height > 0.0 {
                    // Interior generation is a later milestone; for now the
                    // interior key is derived deterministically here and the
                    // interior module (Milestone 6) will flesh out contents.
                    cell.interior_id = interior_id_for(world_x, world_z, seed);
                }
            }

            buf.set_cell(index, cell);
            index += 1;
        }
    }

    buf
}

/// Derive a deterministic interior key for a built cell.
///
/// `0` means "no interior", so built cells (height > 0) always take a nonzero
/// key. Uses a distinct hash domain so the id does not correlate with height
/// or palette draws.
fn interior_id_for(world_x: i64, world_z: i64, seed: u64) -> InteriorId {
    hash_coords(world_x, world_z, seed, domain::INTERIOR)
}

/// Reconstruct the [`InteriorContext`] for a built `Cell` from config.
///
/// The wire `Cell` does not store the context explicitly (it carries the zone
/// affinity, height, palette, and interior id), so this recomputes the
/// exterior→interior bridge deterministically from those fields plus the
/// config's floor mapping. `world_x`/`world_z` are the cell's absolute
/// coordinates: the context's entrance side (which lot edge faces a street) is
/// derived from where the cell sits within its street block. This is what a
/// consumer regenerates when it wants an interior for a cell. Only meaningful
/// when `cell.height > 0`.
#[must_use]
pub fn interior_context_for(
    config: &crate::config::WorldConfig,
    world_x: i64,
    world_z: i64,
    cell: &crate::data::Cell,
) -> crate::layout::InteriorContext {
    // Footprint: the lot spans the zone's street block. The block size is
    // floored at 7 cells (the smallest footprint whose interior always keeps a
    // 2x2 room window free even with a centered 2x2 circulation core), so tiny
    // blocks like Downtown's 4-cell grid never render zero-room stubs.
    let params = config.blended_zone_params(&cell.zone_affinity);
    let side = params.block_size.clamp(7, 32);
    let door_side = door_side_for(world_x, world_z, params.block_size);

    config.interior_context(
        cell.interior_id,
        dominant_zone(&cell.zone_affinity),
        &cell.zone_affinity,
        cell.height,
        side,
        side,
        cell.palette_id,
        door_side,
        config.seed,
    )
}

/// Which edge of the cell's lot faces the nearest street.
///
/// The street grid lies on block boundaries spaced `block_size` cells apart, so
/// the distance to each of the four surrounding roads is the cell's offset
/// within its block. Whichever is nearest (ties resolving toward West, East,
/// North, South) is the entrance side: the main door faces a road.
fn door_side_for(world_x: i64, world_z: i64, block_size: u8) -> DoorSide {
    let b = i64::from(block_size.max(1));
    let rx = world_x.rem_euclid(b);
    let rz = world_z.rem_euclid(b);
    let (dw, de) = (rx, b - rx);
    let (dn, ds) = (rz, b - rz);
    let x_axis = dw.min(de) <= dn.min(ds);
    if x_axis {
        if dw <= de {
            DoorSide::West
        } else {
            DoorSide::East
        }
    } else if dn <= ds {
        DoorSide::North
    } else {
        DoorSide::South
    }
}

/// The zone with the highest affinity for a cell.
///
/// Ties resolve toward the lower variant index, so it is deterministic and
/// matches `examples/viz.rs`'s notion of the dominant zone. Returns [`ZoneType`]
/// rather than an index so callers get a type-checked answer.
fn dominant_zone(affinity: &[f32; crate::zones::ZONE_COUNT]) -> ZoneType {
    let mut best = ZoneType::Downtown;
    let mut best_w = affinity[best as usize];
    for zone in ZoneType::all().into_iter().skip(1) {
        let w = affinity[zone as usize];
        if w > best_w {
            best = zone;
            best_w = w;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::CellFlags;
    use crate::interior::generate_layout;
    use crate::layout::Tile;

    fn fixture() -> (WorldConfig, VoronoiDiagram) {
        let cfg = WorldConfig {
            seed: 1234,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        (cfg, voronoi)
    }

    #[test]
    fn generates_expected_cell_count_and_headers() {
        let (cfg, voronoi) = fixture();
        let buf = generate_chunk(0, 0, &cfg, &voronoi);
        let h = buf.header();
        assert_eq!((h.cx, h.cy), (0, 0));
        assert_eq!(h.chunk_size, cfg.chunk_size);
        assert_eq!(
            h.cell_count,
            u32::from(cfg.chunk_size) * u32::from(cfg.chunk_size)
        );
        assert_eq!(h.seed, cfg.seed);
        // Buffer length must match its on-wire size.
        assert_eq!(buf.as_bytes().len(), 32 + h.cell_count as usize * 40);
    }

    #[test]
    fn generation_is_deterministic() {
        let (cfg, voronoi) = fixture();
        let a = generate_chunk(2, -3, &cfg, &voronoi);
        let b = generate_chunk(2, -3, &cfg, &voronoi);
        assert_eq!(a.as_bytes(), b.as_bytes());
    }

    #[test]
    fn streets_are_zero_height() {
        let (cfg, voronoi) = fixture();
        let buf = generate_chunk(0, 0, &cfg, &voronoi);
        for cell in buf.cells() {
            if cell.flags.contains(CellFlags::IS_STREET) {
                assert_eq!(cell.height, 0.0);
            }
        }
    }

    #[test]
    fn built_cells_have_interiors_and_zone_affinity() {
        let (cfg, voronoi) = fixture();
        let buf = generate_chunk(0, 0, &cfg, &voronoi);
        let mut built = 0;
        for cell in buf.cells() {
            assert!(!cell.zone_affinity.iter().any(|a| *a < 0.0 || a.is_nan()));
            if cell.height > 0.0 {
                built += 1;
                assert_ne!(cell.interior_id, 0);
            }
        }
        // A 32x32 city chunk should contain both streets and buildings.
        let total = buf.cell_count();
        assert!(built > 0 && built < total);
    }

    #[test]
    fn downtown_cells_never_render_zero_room_interiors() {
        // Downtown's raw street block is 4 cells wide, which alone would give
        // interiors too small for a single room. `interior_context_for` floors
        // the footprint at 7x7 and the generator caps the core, so a built
        // downtown lot keeps at least one room per storey. The bridge is tested
        // on a synthetic pure-Downtown cell so the assertion never depends on
        // a seed's street geometry.
        let cfg = WorldConfig {
            seed: 1234,
            ..Default::default()
        };
        let mut affinity = [0.0f32; crate::zones::ZONE_COUNT];
        affinity[crate::zones::ZoneType::Downtown as usize] = 1.0;
        let cell = crate::data::Cell {
            height: 48.0,
            zone_affinity: affinity,
            palette_id: 2,
            flags: CellFlags::NONE,
            _pad: 0,
            interior_id: 1,
        };
        let ctx = interior_context_for(&cfg, 10, 10, &cell);
        assert_eq!(ctx.zone, crate::zones::ZoneType::Downtown);
        assert!(
            ctx.footprint_w >= 7 && ctx.footprint_d >= 7,
            "downtown footprint {}x{} below the roomable floor",
            ctx.footprint_w,
            ctx.footprint_d
        );
        let layout = generate_layout(cell.interior_id, &ctx, &cfg.blueprint_for(ctx.zone));
        assert!(!layout.floors.is_empty());
        for floor in &layout.floors {
            assert!(
                floor.tiles.contains(&Tile::Room),
                "downtown floor has no rooms"
            );
        }
    }

    #[test]
    fn street_flags_match_independent_recomputation() {
        // Cross-chunk edges stay consistent because a cell's street flag is a
        // pure function of its *absolute* world coordinates (via layout_block),
        // never of which chunk generated it. Recompute each cell's street
        // status from the same continuous zone params and require a match.
        let (cfg, voronoi) = fixture();
        let n = i64::from(cfg.chunk_size);
        for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1)] {
            let buf = generate_chunk(cx, cy, &cfg, &voronoi);
            let mut index = 0;
            for local_y in 0..n {
                for local_x in 0..n {
                    let wx = i64::from(cx) * n + local_x;
                    let wz = i64::from(cy) * n + local_y;
                    let cell = buf.get_cell(index);
                    let expected = crate::street::layout_block(
                        wx,
                        wz,
                        &cfg.blended_zone_params(&cell.zone_affinity),
                    );
                    let street_match = CellFlags::IS_STREET.contains(cell.flags)
                        == CellFlags::IS_STREET.contains(expected);
                    assert!(street_match, "street mismatch at world ({wx},{wz})");
                    index += 1;
                }
            }
        }
    }

    #[test]
    fn downtown_taller_than_residential_statistically() {
        // Sample chunks spread widely across the world so we catch both
        // Downtown and Residential Voronoi patches; classify each built cell
        // by whether its Downtown or Residential affinity dominates (index 0
        // vs 1). Downtown's 40..200 height band should beat Residential's
        // 4..18 band in the mean.
        let cfg = WorldConfig {
            seed: 445566,
            voronoi_site_count: 24,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);

        // Chunk indices (cx/cy) spread across the full ±10_000 world span so
        // the sample crosses several Voronoi district cells. (world ≈ cx*32.)
        const SPREAD: [i32; 9] = [-300, -150, -60, -20, 0, 20, 60, 150, 300];

        let mut downtown_heights: Vec<f32> = Vec::new();
        let mut residential_heights: Vec<f32> = Vec::new();

        for &cx in &SPREAD {
            for &cy in &SPREAD {
                let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                for cell in buf.cells() {
                    if cell.height <= 0.0 {
                        continue;
                    }
                    if cell.zone_affinity[0] > cell.zone_affinity[1] {
                        downtown_heights.push(cell.height);
                    } else if cell.zone_affinity[1] > cell.zone_affinity[0] {
                        residential_heights.push(cell.height);
                    }
                }
            }
        }

        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        assert!(
            downtown_heights.len() > 20 && residential_heights.len() > 20,
            "insufficient samples: downtown={} residential={}",
            downtown_heights.len(),
            residential_heights.len()
        );
        let downtown_mean = mean(&downtown_heights);
        let residential_mean = mean(&residential_heights);
        assert!(
            downtown_mean > residential_mean,
            "downtown ({downtown_mean}) not taller than residential ({residential_mean})"
        );
    }

    #[test]
    fn park_dominated_cells_are_flagged() {
        // The `IS_PARK` flag must be set exactly on non-street cells whose Park
        // affinity (index 4) dominates, and must never be set elsewhere.
        let cfg = WorldConfig {
            seed: 445566,
            voronoi_site_count: 24,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        const SPREAD: [i32; 9] = [-300, -150, -60, -20, 0, 20, 60, 150, 300];

        let mut park_cells = 0usize;
        let mut flagged = 0usize;
        for &cx in &SPREAD {
            for &cy in &SPREAD {
                let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                for cell in buf.cells() {
                    // Use the same dominance rule as `generate_chunk`.
                    let is_park = dominant_zone(&cell.zone_affinity) == ZoneType::Park;
                    let is_street = cell.flags.contains(CellFlags::IS_STREET);
                    let has_flag = cell.flags.contains(CellFlags::IS_PARK);
                    if is_park {
                        park_cells += 1;
                        assert_eq!(
                            has_flag, !is_street,
                            "park flag wrong at street={is_street}"
                        );
                        if has_flag {
                            flagged += 1;
                        }
                    } else {
                        assert!(!has_flag, "IS_PARK set on non-park-dominant cell");
                    }
                }
            }
        }
        // The sample must actually contain park-dominant cells, and at least
        // one must be flagged (streets in parks genuinely stay unflagged).
        assert!(park_cells > 0, "no park-dominant cells sampled");
        assert!(flagged > 0, "no IS_PARK flag was ever set");
    }
}
