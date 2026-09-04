//! # layout.rs
//!
//! Interior layout model: the exterior→interior context and the per-zone
//! "blueprint" rule tables that shape room placement (Milestone 9).
//!
//! This module is the bridge between the exterior city and an interior
//! mini-world. It defines:
//!
//! - [`InteriorContext`] — a snapshot of the exterior lot a given interior
//!   belongs to (zone, height, floor count, footprint, palette, seed). Passing
//!   this into the generator (§[`crate::interior::InteriorState`]) is what lets
//!   a downtown skyscraper produce many office floors while a residential house
//!   produces a small home, instead of both hashing to noise.
//! - [`Tile`] — the per-cell kind that makes up a generated interior floor grid
//!   (`#[repr(u8)]`, FFI-friendly).
//! - [`Floor`] / [`InteriorLayout`] — a generated interior: one tile grid per
//!   floor plus metadata.
//! - [`BlueprintRoom`] / [`Blueprint`] — the artist-tunable, data-driven rule
//!   table per zone. Like [`crate::zones::ZoneParams`], blueprints are plain
//!   `Serialize`/`Deserialize`, `#[repr(C)]` fixed-size records so artists tune
//!   them via `WorldConfig` (TOML/JSON) without recompiling and they cross the
//!   FFI boundary unchanged.
//!
//! ## Design
//!
//! - **Context, not magic.** The generator never guesses the building type; it
//!   reads [`InteriorContext`], which is derived deterministically from the
//!   cell's absolute world coordinates (zone via the continuous Voronoi field,
//!   height via the building hash). The layout is therefore reproducible and
//!   cross-chunk-consistent, like the rest of the pipeline.
//! - **Data-driven, FFI-safe blueprints.** Each zone's `Blueprint` is a
//!   fixed-size `#[repr(C)]` record holding a `room_count`-sized prefix of a
//!   fixed room array (mirroring how `Cell` carries a fixed `ZONE_COUNT` affinity
//!   array). Defaults come from [`blueprint_defaults`] as pure data tables;
//!   `WorldConfig.interior_blueprints` holds the tunable copy.
//! - **Separate mini-world.** A layout is a small stand-alone grid keyed by
//!   `InteriorId`, independent of outdoor chunks (§4.4 / §8.1).
//!
//! The actual room-placement algorithm (carving rooms/corridors/doors from the
//! blueprint tables) is a follow-on step; this module pins the *data model* and
//! a deterministic, walled baseline grid that the algorithm and renderers build
//! on.

use serde::{Deserialize, Serialize};

use crate::data::InteriorId;
use crate::zones::ZoneType;

/// Maximum room templates a single zone [`Blueprint`] can hold.
///
/// The blueprint is a fixed-size `#[repr(C)]` record so it can live inside
/// `WorldConfig` and cross the FFI; `room_count` marks how many of the
/// `MAX_BLUEPRINT_ROOMS` slots are live. 8 comfortably fits all five zones'
/// defaults (largest is Downtown at 4).
pub const MAX_BLUEPRINT_ROOMS: usize = 8;

/// Number of `InteriorLayout` floors assumed for worlds units per storey when
/// deriving `floor_count` from building height. Kept as a compile-time default;
/// `WorldConfig.interior_floor_height` overrides it at runtime.
pub const DEFAULT_FLOOR_HEIGHT: f32 = 4.0;

/// Number of `InteriorLayout` floors cap when deriving `floor_count` from
/// building height. `WorldConfig.interior_max_floors` overrides it at runtime.
pub const DEFAULT_MAX_FLOORS: u8 = 64;

// ---------------------------------------------------------------------------
// InteriorContext — exterior lot snapshot fed to the generator
// ---------------------------------------------------------------------------

/// Snapshot of the exterior lot an interior belongs to.
///
/// This is the "information from the exterior map" the generator reacts to:
/// a residential home and a business skyscraper differ here (zone, floors,
/// footprint), so their interiors differ. It is derived deterministically from
/// the cell's absolute world coordinates (zone via the Voronoi field, height
/// and palette via the building hash) and seeded, so the same lot always yields
/// the same context — and thus the same interior.
///
/// `#[repr(C)]` so it can be handed across the FFI boundary for a renderer or
/// tool to inspect or override.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct InteriorContext {
    /// Stable interior key for the built cell (cache key for the mini-world).
    pub id: InteriorId,
    /// Dominant exterior zone, chosen by affinity argmax (see
    /// [`crate::chunk::dominant_zone`]). Picks the blueprint family.
    pub zone: ZoneType,
    /// Blended zone-affinity vector; lets layouts blend near fuzzy borders.
    pub zone_affinity: [f32; crate::zones::ZONE_COUNT],
    /// Exterior building height (world units); 0 means no building.
    pub height: f32,
    /// Number of interior floors derived from `height` (>= 1 for a built lot).
    pub floor_count: u8,
    /// Interior grid width in tiles (footprint derived from the block size).
    pub footprint_w: u8,
    /// Interior grid depth in tiles.
    pub footprint_d: u8,
    /// Exterior facade palette id (rooms tinted to match the building).
    pub palette_id: u8,
    /// World seed used throughout interior derivation.
    pub seed: u64,
}

impl InteriorContext {
    /// Build a context from raw exterior inputs, deriving floor count from
    /// `height`.
    ///
    /// `floor_count` is `ceil(height / floor_height)`, clamped to at least 1
    /// for a built lot (height > 0) and at most `max_floors`. This is the
    /// single place the "height → floors" rule lives so every interior agrees.
    ///
    /// # Many positional fields
    ///
    /// `InteriorContext` is a flat `#[repr(C)]` record; the positional builder
    /// mirrors its field order so callers can pass a struct literal directly
    /// (and the FFI layout stays obvious). Prefer `WorldConfig::interior_context`
    /// for the config-driven path.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // flat FFI record, field-ordered builder
    pub fn new(
        id: InteriorId,
        zone: ZoneType,
        zone_affinity: [f32; crate::zones::ZONE_COUNT],
        height: f32,
        floor_height: f32,
        max_floors: u8,
        footprint_w: u8,
        footprint_d: u8,
        palette_id: u8,
        seed: u64,
    ) -> Self {
        let floor_count = if height <= 0.0 {
            0
        } else {
            let fh = f64::from(floor_height.max(1e-6));
            let n = (f64::from(height) / fh).ceil();
            n.max(1.0).min(f64::from(max_floors)) as u8
        };
        Self {
            id,
            zone,
            zone_affinity,
            height,
            floor_count,
            footprint_w,
            footprint_d,
            palette_id,
            seed,
        }
    }

    /// Whether this lot is buildable (has a positive, non-street footprint).
    #[must_use]
    pub fn is_built(&self) -> bool {
        self.floor_count > 0 && self.footprint_w > 0 && self.footprint_d > 0
    }
}

// ---------------------------------------------------------------------------
// Tile — per-cell kind inside an interior floor grid
// ---------------------------------------------------------------------------

/// A single tile in an interior floor grid.
///
/// `#[repr(u8)]` so a grid can be packed into a flat byte array and shipped
/// over the FFI boundary. Room *kinds* are stored by index so renderers can map
/// them, while the structural tile kinds (void/wall/door) are fixed by the
/// engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
pub enum Tile {
    /// Outside the building volume (unused). Treated as solid.
    Void = 0,
    /// Exterior wall; the sealed boundary of the footprint.
    Wall = 1,
    /// A doorway connecting two traversable tiles (room↔room or room↔corridor).
    Door = 2,
    /// Vertical circulation: stairs/elevator/lobby core.
    Core = 3,
    /// Horizontal circulation connecting rooms (corridor).
    Corridor = 4,
    /// A generic traversable floor tile inside a room (room-kind by index).
    Room = 5,
}

// ---------------------------------------------------------------------------
// BlueprintRoom / Blueprint — per-zone layout rule tables
// ---------------------------------------------------------------------------

/// One room template within a zone's [`Blueprint`].
///
/// A plain data record so room tables are artist-tunable via `WorldConfig`.
/// `kind` is an arbitrary tag the consumer maps to a rendered room (e.g. 0 =
/// living, 1 = kitchen, 2 = bedroom, 3 = office, ...); the engine only treats
/// non-circulation room tiles as `Room` and stores this tag alongside the tile
/// grid for the consumer to interpret.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct BlueprintRoom {
    /// Opaque room-kind tag (semantics belong to the consumer / renderer).
    pub kind: u8,
    /// Relative selection weight when rolling a room for this zone.
    pub weight: f32,
    /// Minimum room grid width in tiles (inclusive).
    pub min_w: u8,
    /// Maximum room grid width in tiles (inclusive).
    pub max_w: u8,
    /// Minimum room grid depth in tiles (inclusive).
    pub min_d: u8,
    /// Maximum room grid depth in tiles (inclusive).
    pub max_d: u8,
}

impl BlueprintRoom {
    /// Convenience constructor keeping call sites short.
    #[must_use]
    pub const fn new(kind: u8, weight: f32, min_w: u8, max_w: u8, min_d: u8, max_d: u8) -> Self {
        Self {
            kind,
            weight,
            min_w,
            max_w,
            min_d,
            max_d,
        }
    }
}

/// The per-zone rule table driving interior layout for that zone.
///
/// Mirrors [`crate::zones::ZoneParams`] for interior generation: plain,
/// `#[repr(C)]`, `Serialize`/`Deserialize` data loaded from `WorldConfig`
/// (TOML/JSON) so artists tune interiors without new code. `Default` per zone
/// gives a sensible starting table ([`blueprint_defaults`]).
///
/// The engine treats these as *rules* — the follow-on `layout` algorithm reads
/// them to carve rooms. The current milestone uses them to drive a
/// deterministic baseline grid: the core placement and the default room tag.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[repr(C)]
pub struct Blueprint {
    /// Structural margin: ring of `Wall` tiles around each floor grid.
    pub margin: u8,
    /// Width of the vertical-circulation core (stairs/elevator) in tiles.
    pub core_size: u8,
    /// Number of live entries in `rooms` (`0..=MAX_BLUEPRINT_ROOMS`).
    pub room_count: u8,
    /// Room templates weighted for this zone; only `room_count` are live.
    pub rooms: [BlueprintRoom; MAX_BLUEPRINT_ROOMS],
}

impl Blueprint {
    /// The live room templates (the `room_count` prefix of `rooms`).
    #[must_use]
    pub fn room_slice(&self) -> &[BlueprintRoom] {
        &self.rooms[..usize::from(self.room_count.min(MAX_BLUEPRINT_ROOMS as u8))]
    }

    /// Whether this blueprint has any room templates.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.room_slice().is_empty()
    }
}

/// Default `Blueprint` for a single [`ZoneType`].
///
/// Room tables are sized to the zone's typical footprint: dense small rooms
/// with a large core downtown, spacious few rooms in homes, and so on. These
/// are starting points, fully overrideable via `WorldConfig`.
#[must_use]
pub fn blueprint_defaults(zone: ZoneType) -> Blueprint {
    // Wall margin + core width, per zone.
    let (margin, core_size) = match zone {
        ZoneType::Downtown => (2, 3),
        ZoneType::Residential => (1, 2),
        ZoneType::Commercial => (1, 2),
        ZoneType::Industrial => (1, 2),
        ZoneType::Park => (1, 1),
    };

    let room_slice: &[BlueprintRoom] = match zone {
        ZoneType::Downtown => &[
            BlueprintRoom::new(10, 3.0, 3, 6, 3, 6), // lobby / lounge
            BlueprintRoom::new(11, 6.0, 3, 4, 3, 4), // open office
            BlueprintRoom::new(12, 4.0, 3, 5, 2, 4), // meeting
            BlueprintRoom::new(13, 3.0, 2, 3, 2, 3), // utility
        ],
        ZoneType::Residential => &[
            BlueprintRoom::new(20, 4.0, 3, 5, 3, 5), // living
            BlueprintRoom::new(21, 3.0, 2, 3, 2, 3), // kitchen
            BlueprintRoom::new(22, 4.0, 3, 4, 3, 4), // bedroom
            BlueprintRoom::new(23, 1.0, 1, 2, 1, 2), // bathroom
        ],
        ZoneType::Commercial => &[
            BlueprintRoom::new(30, 3.0, 4, 6, 3, 5), // retail floor
            BlueprintRoom::new(31, 3.0, 3, 5, 3, 5), // office/flex
            BlueprintRoom::new(32, 2.0, 2, 3, 2, 3), // stockroom
        ],
        ZoneType::Industrial => &[
            BlueprintRoom::new(40, 5.0, 4, 7, 3, 6), // open work bay
            BlueprintRoom::new(41, 2.0, 2, 3, 2, 3), // office/reception
            BlueprintRoom::new(42, 1.0, 1, 2, 1, 2), // washroom
        ],
        ZoneType::Park => &[BlueprintRoom::new(50, 1.0, 2, 3, 2, 3)], // small shed
    };

    // Copy the live rooms into the fixed array's prefix (the rest stay default).
    let mut rooms = [BlueprintRoom::default(); MAX_BLUEPRINT_ROOMS];
    rooms[..room_slice.len()].copy_from_slice(room_slice);

    Blueprint {
        margin,
        core_size,
        room_count: room_slice.len() as u8,
        rooms,
    }
}

/// The default per-zone blueprint table, one entry per [`ZoneType`] variant.
///
/// This is the array `WorldConfig.interior_blueprints` is initialized with,
/// mirroring how `zone_defaults` feeds `WorldConfig.zones`. Index `i` is the
/// blueprint for `ZoneType` variant `i`.
#[must_use]
pub fn default_blueprints() -> [Blueprint; crate::zones::ZONE_COUNT] {
    let mut out = [blueprint_defaults(ZoneType::Downtown); crate::zones::ZONE_COUNT];
    for (i, zone) in ZoneType::all().iter().enumerate() {
        out[i] = blueprint_defaults(*zone);
    }
    out
}

// ---------------------------------------------------------------------------
// Floor / InteriorLayout — the generated result
// ---------------------------------------------------------------------------

/// One floor of an interior: a row-major grid of tiles plus room-kind tags.
///
/// `tiles[floor]` has `width * depth` entries in row-major order (x-major then
/// z-major, matching `ChunkBuffer`'s cell iteration). `room_kinds` carries the
/// opaque [`BlueprintRoom::kind`] for each `Room` tile (one entry per tile,
/// meaningful only where the tile is `Room` or `Corridor`; else 0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Floor {
    /// Grid width in tiles.
    pub width: u8,
    /// Grid depth in tiles.
    pub depth: u8,
    /// Row-major tile grid (`width * depth` entries).
    pub tiles: Vec<Tile>,
    /// Room-kind tag per tile (parallel to `tiles`, 0 where not a room).
    pub kinds: Vec<u8>,
}

impl Floor {
    /// Build an empty all-`Void` floor of the given size.
    #[must_use]
    pub fn empty(width: u8, depth: u8) -> Self {
        let n = usize::from(width) * usize::from(depth);
        Self {
            width,
            depth,
            tiles: vec![Tile::Void; n],
            kinds: vec![0; n],
        }
    }

    /// Index into the row-major grid for `(x, z)`, panicking on OOB.
    #[must_use]
    pub fn index(&self, x: u8, z: u8) -> usize {
        usize::from(z) * usize::from(self.width) + usize::from(x)
    }

    /// Read the tile at `(x, z)`, clamping to `Void` if out of bounds.
    #[must_use]
    pub fn tile(&self, x: u8, z: u8) -> Tile {
        if x >= self.width || z >= self.depth {
            Tile::Void
        } else {
            self.tiles[self.index(x, z)]
        }
    }
}

/// A fully generated interior: one [`Floor`] per level plus metadata.
///
/// This is the output of interior generation (the renderable mini-world),
/// parallel in spirit to `ChunkBuffer` but kept as a separate, owned value
/// keyed by `InteriorId`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InteriorLayout {
    /// The interior key this layout was generated from.
    pub id: InteriorId,
    /// World seed used for generation.
    pub seed: u64,
    /// Context the layout was generated from (floor count, zone, footprint).
    pub context: InteriorContext,
    /// One floor grid per level; `floors[i]` is the `i`-th storey.
    pub floors: Vec<Floor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_derives_floor_count_from_height() {
        // 8u / 4u per floor = exactly 2. ceil keeps partial storeys.
        let ctx = InteriorContext::new(
            1,
            ZoneType::Residential,
            [0.0; 5],
            8.0,
            4.0,
            64,
            8,
            8,
            1,
            42,
        );
        assert_eq!(ctx.floor_count, 2);
        assert!(ctx.is_built());
    }

    #[test]
    fn context_floor_count_respects_cap() {
        // 120u at 4u/floor = 30 floors, capped to 8.
        let ctx =
            InteriorContext::new(1, ZoneType::Downtown, [0.0; 5], 120.0, 4.0, 8, 10, 10, 2, 5);
        assert_eq!(ctx.floor_count, 8);
    }

    #[test]
    fn unbuilt_lot_has_zero_floors() {
        let ctx = InteriorContext::new(1, ZoneType::Park, [0.0; 5], 0.0, 4.0, 64, 4, 4, 0, 5);
        assert_eq!(ctx.floor_count, 0);
        assert!(!ctx.is_built());
    }

    #[test]
    fn blueprint_defaults_cover_all_zones() {
        let all = default_blueprints();
        for zone in ZoneType::all() {
            let bp = all[zone as usize];
            assert!(!bp.is_empty(), "{zone:?} blueprint empty");
            assert!(bp.room_count > 0);
            assert!(bp.room_slice().len() == bp.room_count as usize);
        }
    }

    #[test]
    fn blueprint_room_slice_respects_count() {
        let bp = blueprint_defaults(ZoneType::Residential);
        // room_count is the number of live entries; slice matches it.
        assert_eq!(bp.room_slice().len(), usize::from(bp.room_count));
    }

    #[test]
    fn floor_grid_indexing_is_row_major() {
        let f = Floor::empty(4, 3);
        assert_eq!(f.index(3, 2), 11); // 2*4 + 3
        assert_eq!(f.tile(0, 0), Tile::Void);
        assert_eq!(f.tile(9, 9), Tile::Void); // OOB clamps to Void
    }
}
