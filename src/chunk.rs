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
use crate::region::VoronoiDiagram;
use crate::street;
use crate::zones::zone_params;

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
    let chunk_size = i32::from(config.chunk_size);
    let seed = config.seed;
    let mut buf = ChunkBuffer::new(ChunkId::new(cx, cy), config.chunk_size, seed);

    let mut index = 0usize;
    for local_y in 0..chunk_size {
        for local_x in 0..chunk_size {
            // Absolute world cell coordinates: stable across chunks, so the
            // generated cell at a given world position never changes.
            let world_x = cx * chunk_size + local_x;
            let world_z = cy * chunk_size + local_y;

            let affinity = voronoi.query(f64::from(world_x), f64::from(world_z));
            let params = zone_params(&affinity);

            // Streets first; a street cell never becomes a building.
            let flags = street::layout_block(world_x, world_z, &params);

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
fn interior_id_for(world_x: i32, world_z: i32, seed: u64) -> InteriorId {
    hash_coords(world_x, world_z, seed, domain::INTERIOR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::CellFlags;

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
    fn street_flags_match_independent_recomputation() {
        // Cross-chunk edges stay consistent because a cell's street flag is a
        // pure function of its *absolute* world coordinates (via layout_block),
        // never of which chunk generated it. Recompute each cell's street
        // status from the same continuous zone params and require a match.
        let (cfg, voronoi) = fixture();
        let n = i32::from(cfg.chunk_size);
        for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1)] {
            let buf = generate_chunk(cx, cy, &cfg, &voronoi);
            let mut index = 0;
            for local_y in 0..n {
                for local_x in 0..n {
                    let wx = cx * n + local_x;
                    let wz = cy * n + local_y;
                    let cell = buf.get_cell(index);
                    let expected = crate::street::layout_block(
                        wx,
                        wz,
                        &crate::zones::zone_params(&cell.zone_affinity),
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
}
