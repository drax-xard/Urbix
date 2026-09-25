//! # chunk.rs
//!
//! Per-chunk generation orchestration for the Urbix engine.
//!
//! The world is divided into fixed-size chunks (e.g. 32×32 cells) addressed by
//! integer `(cx, cy)` coordinates. This module generates the full contents of
//! one chunk deterministically, taking a coordinate and the world state and
//! returning a flat array of cells.
//!
//! ## Pipeline (per cell, Milestone 11)
//!
//! 1. Query the continuous Voronoi zone field (`region.rs`) at the cell's
//!    world position to obtain a blended zone-affinity vector.
//! 2. Resolve that into concrete parameters: heights/density blended,
//!    `block_size`/`arterial_every` snapped from the dominant zone.
//! 3. Look up the district frame (`region.rs`) and ask `street.rs` whether
//!    this cell is a street (local or 2-cell arterial); intersections may
//!    become plazas.
//! 4. Non-street cells abutting a street become the 1-cell sidewalk ring.
//! 5. Remaining cells resolve to a lot (`lot.rs`); `building.rs` assigns one
//!    height/palette per lot with CBD, corner, and landmark shaping.
//! 6. Package the result into the `repr(C)` cell record (`data.rs`) with a
//!    lot-keyed `InteriorId`.
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
use crate::hash::{domain, hash_coords, hash_unit};
use crate::layout::DoorSide;
use crate::lot::{block_loc, block_noise, lot_slot};
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
            // Heights/density blend across the fuzzy border; the grid period
            // (block_size/arterial_every) snaps from the dominant zone so
            // transition bands never produce hybrid spacings.
            let params = config.blended_zone_params(&affinity);
            let zone = dominant_zone(&affinity);
            // District fabric: one orientation frame per nearest site, plus
            // the global diagonal boulevards cutting across every grid.
            let frame = voronoi.district_frame_for(world_x as f64, world_z as f64);
            let diags = voronoi.diagonals();

            // Streets first; a street cell never becomes a building. The
            // frame, params, diagonals, and seam flag are shared so neighbour
            // checks below stay consistent.
            let seam = voronoi.is_seam_road(world_x as f64, world_z as f64);
            let info = street::street_info(world_x, world_z, &params, &frame, diags, seam, seed);
            let mut flags = CellFlags::NONE;
            if info.street {
                flags = flags.insert(CellFlags::IS_STREET);
                if info.arterial {
                    flags = flags.insert(CellFlags::IS_ARTERIAL);
                }
            }
            // Flow avenues (Milestone 16): world-space desire paths pave as
            // arterials on top of the lattice answer. Additive and immune to
            // dropout — a flow cell the lattice dropped (or greened) still
            // reads as avenue, and a lattice local upgraded by flow gains the
            // arterial bit. Skipped only when the cell is already an avenue
            // (nothing to add). An empty path list (`flow_path_count == 0`)
            // answers false everywhere, keeping the legacy lattice
            // byte-identical.
            let flow = !(info.street && info.arterial)
                && voronoi.flow_arterial_at(world_x as f64, world_z as f64, config.flow_half_width);
            if flow {
                flags = flags.insert(CellFlags::IS_STREET);
                flags = flags.insert(CellFlags::IS_ARTERIAL);
            } else if !info.street && info.greenway {
                // Dropped segments reborn as linear parks (no-build green).
                flags = flags.insert(CellFlags::IS_GREENWAY);
            }

            // Plazas: a small hashed share of downtown/commercial grid
            // intersections widens into pedestrian ground, and diagonal or
            // flow-avenue crossings earn squares more often (keeps IS_STREET
            // so old renderers still draw pavement).
            let grid_crossing = is_intersection(world_x, world_z, &params, &frame);
            if flags.contains(CellFlags::IS_STREET)
                && ((grid_crossing
                    && (zone == ZoneType::Downtown || zone == ZoneType::Commercial)
                    && hash_unit(world_x, world_z, seed, domain::PLAZA) < 0.02)
                    || ((info.diagonal || flow)
                        && grid_crossing
                        && hash_unit(world_x, world_z, seed, domain::PLAZA) < 0.15))
            {
                flags = flags.insert(CellFlags::IS_PLAZA);
            }

            // Sidewalk ring: non-street, non-green cells abutting any street
            // cell become paved apron (no-build, height 0). Checked with the
            // same full query so chunk edges agree.
            if !flags.contains(CellFlags::IS_STREET)
                && !flags.contains(CellFlags::IS_GREENWAY)
                && abuts_street(
                    world_x,
                    world_z,
                    &params,
                    &frame,
                    diags,
                    voronoi,
                    seed,
                    config.flow_half_width,
                )
            {
                flags = flags.insert(CellFlags::IS_SIDEWALK);
            }

            // A cell dominated by the Park district (and not paved or green)
            // is flagged as greenery. Residential garden blocks (a hashed 15%
            // of residential blocks) read as courtyard green the same way.
            let block = block_loc(world_x, world_z, params.block_size, &frame);
            let garden_block = zone == ZoneType::Residential
                && hash_unit(block.bx, block.bz, seed, domain::BLOCK_NOISE) < 0.15;
            if !flags.contains(CellFlags::IS_STREET)
                && !flags.contains(CellFlags::IS_SIDEWALK)
                && !flags.contains(CellFlags::IS_GREENWAY)
                && (zone == ZoneType::Park || garden_block)
            {
                flags = flags.insert(CellFlags::IS_PARK);
            }

            // Special blocks: a hashed few per zone rewrite the block's
            // program — civic plaza, market sheds, or a tower in a park.
            let special = special_for(block.bx, block.bz, zone, seed);
            if special == SpecialKind::Plaza
                && !flags.contains(CellFlags::IS_STREET)
                && !flags.contains(CellFlags::IS_SIDEWALK)
            {
                flags = flags.insert(CellFlags::IS_PLAZA);
            }

            let mut cell = Cell {
                height: 0.0,
                zone_affinity: affinity,
                palette_id: 0,
                flags,
                _pad: 0,
                interior_id: 0,
            };

            let paved = flags.contains(CellFlags::IS_STREET)
                || flags.contains(CellFlags::IS_SIDEWALK)
                || flags.contains(CellFlags::IS_PLAZA);
            let green =
                flags.contains(CellFlags::IS_PARK) || flags.contains(CellFlags::IS_GREENWAY);
            if !paved && !green {
                let slot = lot_slot(&block, params.block_size, seed);
                let clump = block_noise(block.bx, block.bz, seed);
                let boost = voronoi.cbd_factor(world_x as f64, world_z as f64);
                let rect = crate::lot::lot_rect(&block, params.block_size, seed);
                let (mut height, mut palette) = building::assign_building(
                    slot.lot_id,
                    slot.corner,
                    clump,
                    boost,
                    Some((world_x, world_z)),
                    rect,
                    &params,
                    seed,
                );
                match special {
                    // Market sheds: every lot builds low and tight.
                    SpecialKind::Market => {
                        if height <= 0.0 {
                            (height, palette) = market_shed(slot.lot_id, &params, seed);
                        }
                        height = height.min(10.0);
                    }
                    // Tower in a park: only the middle lot rises (×1.35);
                    // the rest of the block goes green below.
                    SpecialKind::TowerPark if slot.slot != slot.count / 2 => {
                        height = 0.0;
                    }
                    SpecialKind::TowerPark => {
                        if height <= 0.0 {
                            height = 8.0;
                        }
                        height = (height * 1.35).min(params.height_max * 1.6 + 1.0);
                    }
                    SpecialKind::Plaza | SpecialKind::None => {}
                }
                // Landmarks: a hashed share of ordinary and tower lots rises
                // ~1.5× — the wayfinding towers above the street wall.
                // Market sheds stay low (no landmark boost).
                let landmark_ok = special == SpecialKind::None || special == SpecialKind::TowerPark;
                if height > 0.0 && landmark_ok && is_landmark(slot.lot_id, zone, seed) {
                    height = (height * 1.5).min(params.height_max * 1.6 + 1.0);
                }
                // Post-boost slenderness: landmark and tower-park bonuses
                // bypass assign_building's clamp, so re-apply the same
                // footprint cap here — chunky fabric, no spikes.
                if height > 0.0 {
                    height = height.min(building::slenderness_cap(&params, rect.0, rect.1));
                }
                // Tower-park non-tower lots and shed-less empties stay open.
                if height <= 0.0 && special == SpecialKind::TowerPark && slot.slot != slot.count / 2
                {
                    flags = flags.insert(CellFlags::IS_PARK);
                    cell.flags = flags;
                }
                cell.height = height;
                cell.palette_id = palette;
                if height > 0.0 {
                    // Lot-keyed interior: every cell of a lot shares one key
                    // so a building has one interior, not one per cell.
                    cell.interior_id = interior_id_for_lot(block.bx, block.bz, slot.slot, seed);
                }
            }

            buf.set_cell(index, cell);
            index += 1;
        }
    }

    buf
}

/// Whether a world cell sits on a street intersection (both axes on the
/// block boundary in the district frame). Plaza candidates only.
fn is_intersection(
    world_x: i64,
    world_z: i64,
    params: &crate::zones::ZoneParams,
    frame: &crate::lot::DistrictFrame,
) -> bool {
    let loc = block_loc(world_x, world_z, params.block_size, frame);
    loc.rx == 0 && loc.rz == 0
}

/// Whether any 4-neighbour of a world cell is a street (same full query).
/// Used for the sidewalk ring; pure over absolute coords, hence
/// cross-chunk consistent. Dropped segments read as interior/green, never
/// as streets, so the ring hugs real roads only. Neighbour seam flags come
/// from the neighbour's own position (seams are frame-independent), and flow
/// avenues are frame-independent too, so both stay consistent under the
/// center cell's frame.
#[allow(clippy::too_many_arguments)] // full pipeline context per neighbour; bundling hides the query
fn abuts_street(
    world_x: i64,
    world_z: i64,
    params: &crate::zones::ZoneParams,
    frame: &crate::lot::DistrictFrame,
    diagonals: &[crate::lot::Diagonal],
    voronoi: &VoronoiDiagram,
    seed: u64,
    flow_half_width: f32,
) -> bool {
    const DIRS: [(i64, i64); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    DIRS.iter().any(|(dx, dz)| {
        let nx = world_x + dx;
        let nz = world_z + dz;
        let seam = voronoi.is_seam_road(nx as f64, nz as f64);
        street::street_info(nx, nz, params, frame, diagonals, seam, seed).street
            || voronoi.flow_arterial_at(nx as f64, nz as f64, flow_half_width)
    })
}

/// A block's rewritten program, if any.
///
/// Most blocks are `None` (ordinary lots). A hashed few per zone become:
/// - `Plaza`: civic square — the whole interior is pedestrian ground.
/// - `Market`: shed district — every lot builds low (≤ 10 u) and tight.
/// - `TowerPark`: one middle lot rises (×1.35) in a green block.
///
/// Deterministic per `(block, zone, seed)`; probabilities stay low so the
/// ordinary fabric dominates and specials read as accents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpecialKind {
    /// Ordinary lots.
    None,
    /// Civic square.
    Plaza,
    /// Low shed district.
    Market,
    /// Tower in a park.
    TowerPark,
}

fn special_for(bx: i64, bz: i64, zone: ZoneType, seed: u64) -> SpecialKind {
    let r = hash_unit(bx, bz, seed, domain::SPECIAL);
    match zone {
        ZoneType::Downtown => {
            if r < 0.02 {
                SpecialKind::Plaza
            } else if r < 0.04 {
                SpecialKind::TowerPark
            } else {
                SpecialKind::None
            }
        }
        ZoneType::Commercial => {
            if r < 0.02 {
                SpecialKind::Plaza
            } else if r < 0.05 {
                SpecialKind::Market
            } else {
                SpecialKind::None
            }
        }
        ZoneType::Industrial => {
            if r < 0.03 {
                SpecialKind::Market
            } else {
                SpecialKind::None
            }
        }
        ZoneType::Residential | ZoneType::Park => SpecialKind::None,
    }
}

/// Height and palette for a market shed: low (4–10 u), always built.
///
/// Sheds ignore the lot density fate — the whole block builds — but keep a
/// per-lot palette so the row still varies.
fn market_shed(lot_id: u64, params: &crate::zones::ZoneParams, seed: u64) -> (f32, u8) {
    let h = 4.0
        + hash_unit(
            lot_id as i64,
            (lot_id >> 32) as i64,
            seed,
            domain::LOT_HEIGHT,
        ) * 6.0;
    let raw = hash_coords(lot_id as i64, (lot_id >> 32) as i64, seed, domain::PALETTE);
    (h, (raw % (params.palette_count.max(1) as u64)) as u8)
}

/// Whether a lot is a landmark tower for its zone.
///
/// Hashed shares: downtown 4%, commercial 2%, residential/industrial 1%,
/// park never. Deterministic per `(lot_id, seed)`.
fn is_landmark(lot_id: u64, zone: ZoneType, seed: u64) -> bool {
    let p = match zone {
        ZoneType::Downtown => 0.04,
        ZoneType::Commercial => 0.02,
        ZoneType::Residential | ZoneType::Industrial => 0.01,
        ZoneType::Park => return false,
    };
    hash_unit(lot_id as i64, (lot_id >> 32) as i64, seed, domain::LANDMARK) < p
}

/// Derive a deterministic interior key for a lot.
///
/// `0` means "no interior", so built lots always take a nonzero key in
/// practice (the hash is uniform over `u64`; callers treat 0 as none just
/// like the old per-cell key). Keyed on `(block, slot)` — not on the cell —
/// so every cell of one lot shares one interior. Uses a distinct hash domain
/// so the id does not correlate with height or palette draws.
fn interior_id_for_lot(block_bx: i64, block_bz: i64, slot: u8, seed: u64) -> InteriorId {
    hash_coords(
        block_bx.wrapping_mul(16).wrapping_add(i64::from(slot)),
        block_bz,
        seed,
        domain::INTERIOR,
    )
}

/// Reconstruct the [`InteriorContext`] for a built `Cell` from config.
///
/// The wire `Cell` does not store the context explicitly (it carries the zone
/// affinity, height, palette, and interior id), so this recomputes the
/// exterior→interior bridge deterministically from those fields plus the
/// config's floor mapping. `world_x`/`world_z` are the cell's absolute
/// coordinates: lot geometry (pack rect, corner, entrance side) and the
/// building role re-derive from the same framed block pipeline generation
/// uses, so every cell of one lot reconstructs the same context. This is
/// what a consumer regenerates when it wants an interior for a cell. Only
/// meaningful when `cell.height > 0`.
#[must_use]
pub fn interior_context_for(
    config: &crate::config::WorldConfig,
    voronoi: &VoronoiDiagram,
    world_x: i64,
    world_z: i64,
    cell: &crate::data::Cell,
) -> crate::layout::InteriorContext {
    let params = config.blended_zone_params(&cell.zone_affinity);
    let zone = dominant_zone(&cell.zone_affinity);
    let frame = voronoi.district_frame_for(world_x as f64, world_z as f64);
    let block = crate::lot::block_loc(world_x, world_z, params.block_size, &frame);
    let slot = crate::lot::lot_slot(&block, params.block_size, config.seed);
    // Truthful footprint: the lot's pack-rect size, bounded so degenerate
    // configs stay renderable.
    let (rw, rd) = crate::lot::lot_rect(&block, params.block_size, config.seed);
    let (footprint_w, footprint_d) = (rw.clamp(3, 64), rd.clamp(3, 64));
    let door_side = door_side_for(world_x, world_z, params.block_size);
    let role = {
        let special = special_for(block.bx, block.bz, zone, config.seed);
        match special {
            SpecialKind::Market => crate::layout::BuildingRole::Market,
            SpecialKind::TowerPark => crate::layout::BuildingRole::TowerPark,
            SpecialKind::Plaza | SpecialKind::None
                if is_landmark(slot.lot_id, zone, config.seed) =>
            {
                crate::layout::BuildingRole::Landmark
            }
            SpecialKind::Plaza | SpecialKind::None => crate::layout::BuildingRole::Ordinary,
        }
    };

    config.interior_context(
        cell.interior_id,
        zone,
        &cell.zone_affinity,
        cell.height,
        footprint_w,
        footprint_d,
        cell.palette_id,
        door_side,
        config.seed,
        slot.corner,
        role,
        secondary_zone(&cell.zone_affinity, zone),
    )
}

/// Affinity runner-up behind `dominant` (ties toward the lower index, like
/// [`dominant_zone`]). Drives mixed-use ground floors; never equals the
/// dominant zone unless the vector is degenerate.
fn secondary_zone(affinity: &[f32; crate::zones::ZONE_COUNT], dominant: ZoneType) -> ZoneType {
    let mut best = if dominant == ZoneType::Downtown {
        ZoneType::Residential
    } else {
        ZoneType::Downtown
    };
    let mut best_w = f32::NEG_INFINITY;
    for zone in ZoneType::all() {
        if zone == dominant {
            continue;
        }
        let w = affinity[zone as usize];
        if w > best_w {
            best_w = w;
            best = zone;
        }
    }
    best
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
            // Paved ground — streets, sidewalks, plazas — never builds.
            if cell.flags.contains(CellFlags::IS_STREET)
                || cell.flags.contains(CellFlags::IS_SIDEWALK)
            {
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
        // `interior_context_for` reports the lot's true pack rect and the
        // generator caps the core, so a built lot keeps at least one room per
        // storey. The bridge is tested on a synthetic pure-Downtown cell so
        // the assertion never depends on a seed's street geometry.
        let cfg = WorldConfig {
            seed: 1234,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
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
        let ctx = interior_context_for(&cfg, &voronoi, 10, 10, &cell);
        assert_eq!(ctx.zone, crate::zones::ZoneType::Downtown);
        // Truthful lot geometry: footprint matches the pack rect at (10,10).
        let params = cfg.blended_zone_params(&cell.zone_affinity);
        let frame = voronoi.district_frame_for(10.0, 10.0);
        let block = crate::lot::block_loc(10, 10, params.block_size, &frame);
        let (rw, rd) = crate::lot::lot_rect(&block, params.block_size, cfg.seed);
        assert_eq!(
            (ctx.footprint_w, ctx.footprint_d),
            (rw.clamp(3, 64), rd.clamp(3, 64))
        );
        assert_ne!(ctx.secondary_zone, ctx.zone);
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
        // Cross-chunk edges stay consistent because a cell's street flags are
        // a pure function of its *absolute* world coordinates in the district
        // frame (via street_info) plus frame-independent world-space avenues
        // (diagonals, seams, Milestone 16 flow paths), never of which chunk
        // generated it. Recompute each cell's full street answer from the
        // same continuous zone params + frame + diagonals + flow and require
        // a match on every bit the pipeline sets (street, arterial,
        // greenway). (Plazas keep IS_STREET, so the street bit still matches
        // the base answer.)
        let (cfg, voronoi) = fixture();
        let diags = voronoi.diagonals().to_vec();
        let n = i64::from(cfg.chunk_size);
        for (cx, cy) in [(0, 0), (1, 0), (0, 1), (-1, -1)] {
            let buf = generate_chunk(cx, cy, &cfg, &voronoi);
            let mut index = 0;
            for local_y in 0..n {
                for local_x in 0..n {
                    let wx = i64::from(cx) * n + local_x;
                    let wz = i64::from(cy) * n + local_y;
                    let cell = buf.get_cell(index);
                    let frame = voronoi.district_frame_for(wx as f64, wz as f64);
                    let seam = voronoi.is_seam_road(wx as f64, wz as f64);
                    let expected = crate::street::street_info(
                        wx,
                        wz,
                        &cfg.blended_zone_params(&cell.zone_affinity),
                        &frame,
                        &diags,
                        seam,
                        cfg.seed,
                    );
                    // Flow avenues OR onto the lattice answer (and win over
                    // greenways); skipped only for existing avenues.
                    let flow = !(expected.street && expected.arterial)
                        && voronoi.flow_arterial_at(wx as f64, wz as f64, cfg.flow_half_width);
                    assert_eq!(
                        cell.flags.contains(CellFlags::IS_STREET),
                        expected.street || flow,
                        "street mismatch at world ({wx},{wz})"
                    );
                    assert_eq!(
                        cell.flags.contains(CellFlags::IS_ARTERIAL),
                        expected.arterial || flow,
                        "arterial mismatch at world ({wx},{wz})"
                    );
                    assert_eq!(
                        cell.flags.contains(CellFlags::IS_GREENWAY),
                        !flow && !expected.street && expected.greenway,
                        "greenway mismatch at world ({wx},{wz})"
                    );
                    index += 1;
                }
            }
        }
    }

    #[test]
    fn dropout_greenways_and_diagonals_texture_the_city() {
        // A wide sample must contain dropped greenways (height 0, green
        // flag). Diagonal boulevards are sampled where they provably run:
        // chunks around each boulevard's closest approach to the origin.
        let cfg = WorldConfig {
            seed: 445566,
            voronoi_site_count: 24,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        let diags = voronoi.diagonals().to_vec();
        assert_eq!(diags.len(), 2);
        const SPREAD: [i32; 5] = [-60, -20, 0, 20, 60];
        let mut greenway = 0usize;
        for &cx in &SPREAD {
            for &cy in &SPREAD {
                let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                for cell in buf.cells() {
                    if cell.flags.contains(CellFlags::IS_GREENWAY) {
                        greenway += 1;
                        assert_eq!(cell.height, 0.0);
                        assert_eq!(cell.interior_id, 0);
                    }
                }
            }
        }
        assert!(greenway > 0, "no greenways sampled");
        // Boulevard-proximal chunks: closest point to the origin per diagonal.
        let n = i64::from(cfg.chunk_size);
        for d in &diags {
            let nx = -d.angle_rad.sin();
            let nz = d.angle_rad.cos();
            let px = d.offset * nx;
            let pz = d.offset * nz;
            let ccx = (px as i64).div_euclid(n) as i32;
            let ccz = (pz as i64).div_euclid(n) as i32;
            let mut diagonal = 0usize;
            for cx in ccx - 1..=ccx + 1 {
                for cy in ccz - 1..=ccz + 1 {
                    let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                    let mut index = 0;
                    for ly in 0..n {
                        for lx in 0..n {
                            let wx = i64::from(cx) * n + lx;
                            let wz = i64::from(cy) * n + ly;
                            let cell = buf.get_cell(index);
                            index += 1;
                            let frame = voronoi.district_frame_for(wx as f64, wz as f64);
                            let seam = voronoi.is_seam_road(wx as f64, wz as f64);
                            let info = crate::street::street_info(
                                wx,
                                wz,
                                &cfg.blended_zone_params(&cell.zone_affinity),
                                &frame,
                                &diags,
                                seam,
                                cfg.seed,
                            );
                            if info.diagonal && info.street {
                                diagonal += 1;
                                assert!(cell.flags.contains(CellFlags::IS_ARTERIAL));
                            }
                        }
                    }
                }
            }
            assert!(
                diagonal > 0,
                "boulevard at offset {} paves nothing",
                d.offset
            );
        }
    }

    #[test]
    fn seam_parkways_pave_district_borders() {
        // Wherever the Voronoi seam runs, cells read as arterial streets —
        // the border becomes a parkway both grids tee into, never a tear.
        // The test locates a real border first (midpoint of a close,
        // unowned site pair), then checks the parkway around it.
        let cfg = WorldConfig {
            seed: 445566,
            voronoi_site_count: 24,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        let n = i64::from(cfg.chunk_size);
        let sites = voronoi.sites().to_vec();
        let mut checked = 0;
        for (i, a) in sites.iter().enumerate() {
            let mut best = f64::INFINITY;
            let mut nb = 0usize;
            for (j, b) in sites.iter().enumerate() {
                if i == j {
                    continue;
                }
                let d2 = (a.x - b.x).powi(2) + (a.y - b.y).powi(2);
                if d2 < best {
                    best = d2;
                    nb = j;
                }
            }
            let gap = best.sqrt();
            if !(500.0..=3000.0).contains(&gap) {
                continue;
            }
            let b = &sites[nb];
            let mx = (a.x + b.x) / 2.0;
            let mz = (a.y + b.y) / 2.0;
            // Skip borders a third site owns (same guard as region tests).
            let owned_by_third = sites.iter().enumerate().any(|(k, s)| {
                k != i
                    && k != nb
                    && ((s.x - mx).powi(2) + (s.y - mz).powi(2)).sqrt() < gap / 2.0 - 1e-9
            });
            if owned_by_third || !voronoi.is_seam_road(mx, mz) {
                continue;
            }
            let ccx = (mx as i64).div_euclid(n) as i32;
            let ccz = (mz as i64).div_euclid(n) as i32;
            let mut seam_cells = 0usize;
            for cx in ccx - 1..=ccx + 1 {
                for cy in ccz - 1..=ccz + 1 {
                    let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                    let mut index = 0;
                    for ly in 0..n {
                        for lx in 0..n {
                            let wx = i64::from(cx) * n + lx;
                            let wz = i64::from(cy) * n + ly;
                            let cell = buf.get_cell(index);
                            index += 1;
                            if voronoi.is_seam_road(wx as f64, wz as f64) {
                                seam_cells += 1;
                                assert!(
                                    cell.flags.contains(CellFlags::IS_STREET),
                                    "seam tear at world ({wx},{wz})"
                                );
                                assert!(cell.flags.contains(CellFlags::IS_ARTERIAL));
                                assert_eq!(cell.height, 0.0);
                            }
                        }
                    }
                }
            }
            assert!(seam_cells >= 10, "seam parkway too thin ({seam_cells})");
            checked += 1;
        }
        assert!(checked > 0, "no seam borders sampled");
    }

    #[test]
    fn special_blocks_follow_their_program() {
        // Plaza blocks build nothing (all interior is pedestrian ground);
        // market blocks cap at shed height; tower-park blocks raise exactly
        // one lot.
        use std::collections::{HashMap, HashSet};
        let cfg = WorldConfig {
            seed: 445566,
            voronoi_site_count: 24,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        const SPREAD: [i32; 7] = [-150, -60, -20, 0, 20, 60, 150];
        // (block, chunk-frame params) → cells. Frame/params vary per cell, so
        // regroup by lattice block recomputed per cell (public API only).
        let mut plaza_blocks: HashSet<(i64, i64)> = HashSet::new();
        let mut market_tall = false;
        let mut towerpark_lots: HashMap<(i64, i64), HashSet<u64>> = HashMap::new();
        for &cx in &SPREAD {
            for &cy in &SPREAD {
                let buf = generate_chunk(cx, cy, &cfg, &voronoi);
                let n = i64::from(cfg.chunk_size);
                let mut index = 0;
                for ly in 0..n {
                    for lx in 0..n {
                        let wx = i64::from(cx) * n + lx;
                        let wz = i64::from(cy) * n + ly;
                        let cell = buf.get_cell(index);
                        index += 1;
                        let frame = voronoi.district_frame_for(wx as f64, wz as f64);
                        let params = cfg.blended_zone_params(&cell.zone_affinity);
                        let block = crate::lot::block_loc(wx, wz, params.block_size, &frame);
                        let zone = dominant_zone(&cell.zone_affinity);
                        match special_for(block.bx, block.bz, zone, cfg.seed) {
                            SpecialKind::Plaza => {
                                plaza_blocks.insert((block.bx, block.bz));
                                if !cell.flags.contains(CellFlags::IS_STREET)
                                    && !cell.flags.contains(CellFlags::IS_SIDEWALK)
                                {
                                    assert_eq!(cell.height, 0.0);
                                    assert!(cell.flags.contains(CellFlags::IS_PLAZA));
                                }
                            }
                            SpecialKind::Market => {
                                if cell.height > 10.0 {
                                    market_tall = true;
                                }
                            }
                            SpecialKind::TowerPark => {
                                if cell.height > 0.0 {
                                    towerpark_lots
                                        .entry((block.bx, block.bz))
                                        .or_default()
                                        .insert(cell.interior_id);
                                }
                            }
                            SpecialKind::None => {}
                        }
                    }
                }
            }
        }
        assert!(!plaza_blocks.is_empty(), "no plaza blocks sampled");
        assert!(!market_tall, "market block exceeds shed height");
        assert!(!towerpark_lots.is_empty(), "no tower-park blocks sampled");
        for ((bx, bz), lots) in &towerpark_lots {
            assert_eq!(
                lots.len(),
                1,
                "tower-park block ({bx},{bz}) raises {lots:?}"
            );
        }
    }

    #[test]
    fn lot_mates_share_interior_and_palette() {
        // Every cell of one lot carries one interior key and one palette:
        // group a chunk's built cells by interior_id and require uniformity.
        use std::collections::HashMap;
        let (cfg, voronoi) = fixture();
        let buf = generate_chunk(3, -2, &cfg, &voronoi);
        let mut groups: HashMap<u64, (u8, usize)> = HashMap::new();
        for cell in buf.cells() {
            if cell.height <= 0.0 {
                continue;
            }
            let e = groups
                .entry(cell.interior_id)
                .or_insert((cell.palette_id, 0));
            assert_eq!(e.0, cell.palette_id, "lot splits palette");
            e.1 += 1;
        }
        assert!(!groups.is_empty(), "no built lots sampled");
        // Lots span multiple cells: at least one lot has mates.
        assert!(
            groups.values().any(|(_, n)| *n > 1),
            "no multi-cell lot found"
        );
    }

    #[test]
    fn sidewalks_ring_the_streets_and_arterials_exist() {
        // The sidewalk apron must exist wherever streets run, and the
        // per-zone arterial lattice must produce flagged avenues.
        let (cfg, voronoi) = fixture();
        let mut sidewalk = 0usize;
        let mut arterial = 0usize;
        let mut street = 0usize;
        for (cx, cy) in [(0, 0), (5, -3), (-4, 7)] {
            let buf = generate_chunk(cx, cy, &cfg, &voronoi);
            for cell in buf.cells() {
                if cell.flags.contains(CellFlags::IS_STREET) {
                    street += 1;
                }
                if cell.flags.contains(CellFlags::IS_SIDEWALK) {
                    sidewalk += 1;
                    assert_eq!(cell.height, 0.0);
                }
                if cell.flags.contains(CellFlags::IS_ARTERIAL) {
                    arterial += 1;
                    assert!(cell.flags.contains(CellFlags::IS_STREET));
                }
            }
        }
        assert!(
            street > 0 && sidewalk > 0,
            "street={street} sidewalk={sidewalk}"
        );
        assert!(arterial > 0, "no arterial avenues generated");
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
        // The `IS_PARK` flag must be set on unpaved cells whose Park affinity
        // (index 4) dominates. Residential garden blocks may also flag park;
        // anywhere else the flag must stay off. Paved cells (street/sidewalk)
        // never carry it.
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
                let n = i64::from(cfg.chunk_size);
                let mut index = 0;
                for ly in 0..n {
                    for lx in 0..n {
                        let wx = i64::from(cx) * n + lx;
                        let wz = i64::from(cy) * n + ly;
                        let cell = buf.get_cell(index);
                        index += 1;
                        // Use the same dominance rule as `generate_chunk`.
                        let zone = dominant_zone(&cell.zone_affinity);
                        let paved = cell.flags.contains(CellFlags::IS_STREET)
                            || cell.flags.contains(CellFlags::IS_SIDEWALK);
                        let has_flag = cell.flags.contains(CellFlags::IS_PARK);
                        assert!(!paved || !has_flag, "paved cell flagged as park");
                        if zone == ZoneType::Park {
                            park_cells += 1;
                            // Park green reads as IS_PARK, or as a greenway
                            // where a street dropped out of the park grid.
                            let green_any = has_flag || cell.flags.contains(CellFlags::IS_GREENWAY);
                            assert_eq!(green_any, !paved, "park flag wrong at paved={paved}");
                            if green_any {
                                flagged += 1;
                            }
                        } else if zone == ZoneType::Residential {
                            // Garden blocks: flagged only when unpaved.
                            if has_flag {
                                assert!(!paved);
                                flagged += 1;
                            }
                        } else if has_flag {
                            // Tower-park green: non-tower lots of a Downtown
                            // tower-park block read as park. Recompute the
                            // block program and require exactly that.
                            assert!(!paved);
                            let frame = voronoi.district_frame_for(wx as f64, wz as f64);
                            let params = cfg.blended_zone_params(&cell.zone_affinity);
                            let block = crate::lot::block_loc(wx, wz, params.block_size, &frame);
                            assert_eq!(
                                special_for(block.bx, block.bz, zone, cfg.seed),
                                SpecialKind::TowerPark,
                                "IS_PARK on {zone:?}-dominant cell outside tower-park"
                            );
                            let slot = crate::lot::lot_slot(&block, params.block_size, cfg.seed);
                            assert_ne!(slot.slot, slot.count / 2, "tower lot flagged park");
                            flagged += 1;
                        }
                    }
                }
            }
        }
        // The sample must actually contain park-dominant cells, and at least
        // one must be flagged (streets in parks genuinely stay unflagged).
        assert!(park_cells > 0, "no park-dominant cells sampled");
        assert!(flagged > 0, "no IS_PARK flag was ever set");
    }

    // --- Milestone 16: flow avenues in the chunk pipeline ---

    /// World cell at the midpoint of a diagram's first desire path, rounded
    /// to the nearest integer cell (within ~0.71 of the segment, so inside
    /// the default half-width).
    fn first_path_midpoint_cell(voronoi: &VoronoiDiagram) -> (i64, i64) {
        let p = voronoi
            .flow_paths()
            .first()
            .expect("diagram has flow paths");
        (
            ((p.ax + p.bx) / 2.0).round() as i64,
            ((p.ay + p.by) / 2.0).round() as i64,
        )
    }

    #[test]
    fn flow_midpoint_cell_paves_as_arterial_street() {
        // A desire-path midpoint paves unconditionally: street + arterial,
        // height 0, no interior — regardless of what the lattice says there.
        let cfg = WorldConfig {
            seed: 445566,
            ..Default::default()
        };
        let voronoi = VoronoiDiagram::generate(cfg.seed, cfg.voronoi_site_count);
        let (wx, wz) = first_path_midpoint_cell(&voronoi);
        let n = i64::from(cfg.chunk_size);
        let (cx, cy) = (wx.div_euclid(n) as i32, wz.div_euclid(n) as i32);
        let buf = generate_chunk(cx, cy, &cfg, &voronoi);
        let (lx, ly) = (wx.rem_euclid(n), wz.rem_euclid(n));
        let cell = buf.get_cell((ly * n + lx) as usize);
        assert!(
            cell.flags.contains(CellFlags::IS_STREET),
            "flow midpoint not a street at ({wx},{wz})"
        );
        assert!(
            cell.flags.contains(CellFlags::IS_ARTERIAL),
            "flow midpoint not an arterial at ({wx},{wz})"
        );
        assert_eq!(cell.height, 0.0);
        assert_eq!(cell.interior_id, 0);
    }

    #[test]
    fn flow_wiring_is_additive_over_the_legacy_lattice() {
        // Scan seeds for a desire-path midpoint the legacy lattice leaves
        // non-arterial: the flow chunk must upgrade exactly that cell to an
        // arterial street (additive — never removing lattice pavement).
        let mut found = 0;
        for seed in 0..30u64 {
            let flow_cfg = WorldConfig {
                seed,
                ..Default::default()
            };
            let legacy_cfg = WorldConfig {
                seed,
                flow_path_count: 0,
                ..Default::default()
            };
            assert!(legacy_cfg.is_valid());
            let d_flow = VoronoiDiagram::generate(flow_cfg.seed, flow_cfg.voronoi_site_count);
            let d_legacy = VoronoiDiagram::generate_with_config(&legacy_cfg);
            // Same seed, paths disabled: identical sites, empty path list.
            assert!(d_legacy.flow_paths().is_empty());
            let (wx, wz) = first_path_midpoint_cell(&d_flow);
            let n = i64::from(flow_cfg.chunk_size);
            let (cx, cy) = (wx.div_euclid(n) as i32, wz.div_euclid(n) as i32);
            let flow_buf = generate_chunk(cx, cy, &flow_cfg, &d_flow);
            let legacy_buf = generate_chunk(cx, cy, &legacy_cfg, &d_legacy);
            let (lx, ly) = (wx.rem_euclid(n), wz.rem_euclid(n));
            let idx = (ly * n + lx) as usize;
            let flow_cell = flow_buf.get_cell(idx);
            let legacy_cell = legacy_buf.get_cell(idx);
            assert!(flow_cell.flags.contains(CellFlags::IS_ARTERIAL));
            if !legacy_cell.flags.contains(CellFlags::IS_ARTERIAL) {
                // The upgrade case: lattice left it local/open, flow paved it.
                assert!(flow_cell.flags.contains(CellFlags::IS_STREET));
                assert_eq!(flow_cell.height, 0.0);
                found += 1;
            }
            // Pavement is never removed: every legacy street stays a street.
            for i in 0..legacy_buf.cell_count() {
                if legacy_buf.get_cell(i).flags.contains(CellFlags::IS_STREET) {
                    assert!(
                        flow_buf.get_cell(i).flags.contains(CellFlags::IS_STREET),
                        "seed {seed}: flow removed lattice pavement at cell {i}"
                    );
                }
            }
        }
        assert!(found > 0, "no additive flow upgrade sampled");
    }

    #[test]
    fn flow_avenues_feed_the_sidewalk_ring() {
        // A cell abutting a flow avenue (and nothing else) reads as sidewalk
        // in the flow chunk but not in the legacy chunk. `abuts_street` is
        // exercised through the same params/frame the pipeline uses for the
        // center cell, so the assertion covers the real wiring, not a model.
        let mut found = 0;
        for seed in 0..50u64 {
            let flow_cfg = WorldConfig {
                seed,
                ..Default::default()
            };
            let legacy_cfg = WorldConfig {
                seed,
                flow_path_count: 0,
                ..Default::default()
            };
            let d_flow = VoronoiDiagram::generate(flow_cfg.seed, flow_cfg.voronoi_site_count);
            let d_legacy = VoronoiDiagram::generate_with_config(&legacy_cfg);
            let (mx, mz) = first_path_midpoint_cell(&d_flow);
            let n = i64::from(flow_cfg.chunk_size);
            let (cx, cy) = (mx.div_euclid(n) as i32, mz.div_euclid(n) as i32);
            let flow_buf = generate_chunk(cx, cy, &flow_cfg, &d_flow);
            let legacy_buf = generate_chunk(cx, cy, &legacy_cfg, &d_legacy);
            const DIRS: [(i64, i64); 4] = [(1, 0), (-1, 0), (0, 1), (0, -1)];
            for (dx, dz) in DIRS {
                let (nx, nz) = (mx + dx, mz + dz);
                let (lx, lz) = (nx.rem_euclid(n), nz.rem_euclid(n));
                // Neighbour must land in the same chunk (else the cell index
                // below addresses the wrong chunk).
                if nx.div_euclid(n) as i32 != cx || nz.div_euclid(n) as i32 != cy {
                    continue;
                }
                // Skip neighbours the avenue itself paves.
                if d_flow.flow_arterial_at(nx as f64, nz as f64, flow_cfg.flow_half_width) {
                    continue;
                }
                // Replicate the pipeline's center-cell inputs exactly.
                let affinity = d_flow.query(nx as f64, nz as f64);
                let params = flow_cfg.blended_zone_params(&affinity);
                let frame = d_flow.district_frame_for(nx as f64, nz as f64);
                let diags = d_flow.diagonals();
                let legacy_abuts = abuts_street(
                    nx,
                    nz,
                    &params,
                    &frame,
                    diags,
                    &d_legacy,
                    seed,
                    legacy_cfg.flow_half_width,
                );
                if legacy_abuts {
                    continue; // lattice already rings it; not a flow proof
                }
                let flow_abuts = abuts_street(
                    nx,
                    nz,
                    &params,
                    &frame,
                    diags,
                    &d_flow,
                    seed,
                    flow_cfg.flow_half_width,
                );
                if !flow_abuts {
                    continue;
                }
                let idx = (lz * n + lx) as usize;
                let flow_cell = flow_buf.get_cell(idx);
                let legacy_cell = legacy_buf.get_cell(idx);
                // The ring must actually render: sidewalk in flow, absent in
                // legacy (greenways and plazas are excluded — they never
                // carry the ring by construction).
                if flow_cell.flags.contains(CellFlags::IS_GREENWAY)
                    || flow_cell.flags.contains(CellFlags::IS_PLAZA)
                {
                    continue;
                }
                assert!(
                    flow_cell.flags.contains(CellFlags::IS_SIDEWALK),
                    "seed {seed}: flow-abutting cell not ringed at ({nx},{nz})"
                );
                assert!(
                    !legacy_cell.flags.contains(CellFlags::IS_SIDEWALK),
                    "seed {seed}: legacy cell already ringed at ({nx},{nz})"
                );
                found += 1;
            }
        }
        assert!(found > 0, "no flow-only sidewalk ring sampled");
    }
}
