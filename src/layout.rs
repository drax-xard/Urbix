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

/// Which of the four lot edges faces a street: the main entrance is placed on
/// that side so an interior always opens toward the road.
///
/// Derived at chunk generation time from the cell's position within its street
/// block (the nearest boundary road), then recorded in [`InteriorContext`] so
/// the generator and renderers agree without re-deriving it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum DoorSide {
    /// Main entrance on the west edge (negative x).
    West = 0,
    /// Main entrance on the east edge (positive x).
    East = 1,
    /// Main entrance on the north edge (negative z).
    North = 2,
    /// Main entrance on the south edge (positive z).
    South = 3,
}

/// What a lot's building is, beyond its zone.
///
/// Recomputed pure from world coordinates alongside the other context fields
/// (`chunk::interior_context_for`), so the generator can give a market shed,
/// a tower-park tower, or a landmark a different program from an ordinary
/// lot on the same street.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum BuildingRole {
    /// Ordinary lot: the zone blueprint's default program.
    Ordinary = 0,
    /// Wayfinding tower: heightened version of the ordinary program.
    Landmark = 1,
    /// Market shed district: low, fully-built rows.
    Market = 2,
    /// Single tower rising in a green block.
    TowerPark = 3,
}

/// A storey's role within its building.
///
/// Ground floors face the street (entrance, lobby, retail-capable rooms);
/// typical floors repeat one layout (drawn once, cloned); top floors close
/// the building (expanded core, no entrance). Single-storey buildings are
/// all Ground.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum FloorRole {
    /// Street level: entrance door, lobby halo, retail-capable rooms.
    Ground = 0,
    /// Repeated middle floors: one layout generated once, then cloned.
    Typical = 1,
    /// Crown: expanded mechanical core, no entrance.
    Top = 2,
}

/// Role of storey `floor` in a building of `floor_count` storeys.
///
/// Index 0 is always Ground; the last index (when `floor_count >= 2`) is
/// Top; everything between is Typical. A zero `floor_count` still reports
/// Ground so degenerate lots yield one usable sealed floor.
///
/// ## Example
///
/// ```
/// use urbix::layout::{floor_role, FloorRole};
/// assert_eq!(floor_role(0, 5), FloorRole::Ground);
/// assert_eq!(floor_role(2, 5), FloorRole::Typical);
/// assert_eq!(floor_role(4, 5), FloorRole::Top);
/// assert_eq!(floor_role(0, 1), FloorRole::Ground);
/// ```
#[must_use]
pub const fn floor_role(floor: u8, floor_count: u8) -> FloorRole {
    if floor == 0 || floor_count <= 1 {
        FloorRole::Ground
    } else if floor + 1 >= floor_count {
        FloorRole::Top
    } else {
        FloorRole::Typical
    }
}

/// Snapshot of the exterior lot an interior belongs to.
///
/// This is the "information from the exterior map" the generator reacts to:
/// a residential home and a business skyscraper differ here (zone, floors,
/// footprint), so their interiors differ. It is derived deterministically from
/// the cell's absolute world coordinates (zone via the Voronoi field, height
/// and palette via the building hash) and seeded, so the same lot always yields
/// the same context — and thus the same interior.
///
/// `footprint_w/d` is the lot's true pack-rect size in tiles (see
/// `crate::lot::lot_rect`), not a square block estimate, so narrow lots get
/// narrow interiors. `corner`, `frontage_depth`, and `building_role` let the
/// generator shape corner towers, shallow shopfronts, and market sheds
/// differently; `secondary_zone` (affinity runner-up) drives mixed-use
/// ground floors.
///
/// `#[repr(C)]` so it can be handed across the FFI boundary for a renderer or
/// tool to inspect or override. New fields are always appended.
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
    /// Interior grid width in tiles (lot pack-rect width).
    pub footprint_w: u8,
    /// Interior grid depth in tiles (lot pack-rect depth).
    pub footprint_d: u8,
    /// Exterior facade palette id (rooms tinted to match the building).
    pub palette_id: u8,
    /// Lot edge facing a street; the main entrance is placed on this side.
    pub door_side: DoorSide,
    /// World seed used throughout interior derivation.
    pub seed: u64,
    /// The lot touches streets on two axes (corner tower treatment).
    pub corner: bool,
    /// Lot depth in tiles perpendicular to the entrance edge (shopfront
    /// depth, lobby depth); derived from the rect and `door_side`.
    pub frontage_depth: u8,
    /// What the lot's building is beyond its zone (market, tower, landmark).
    pub building_role: BuildingRole,
    /// Affinity runner-up behind `zone`; drives mixed-use ground floors.
    pub secondary_zone: ZoneType,
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
        door_side: DoorSide,
        seed: u64,
        corner: bool,
        building_role: BuildingRole,
        secondary_zone: ZoneType,
    ) -> Self {
        let floor_count = if height <= 0.0 {
            0
        } else {
            let fh = f64::from(floor_height.max(1e-6));
            let n = (f64::from(height) / fh).ceil();
            n.max(1.0).min(f64::from(max_floors)) as u8
        };
        // Frontage depth: lot extent perpendicular to the entrance edge.
        let frontage_depth = match door_side {
            DoorSide::West | DoorSide::East => footprint_w,
            DoorSide::North | DoorSide::South => footprint_d,
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
            door_side,
            seed,
            corner,
            frontage_depth,
            building_role,
            secondary_zone,
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

/// Room adjacency/program tag bits for [`BlueprintRoom::tags`].
///
/// Tags steer placement without the engine knowing room semantics: `WET`
/// rooms snap to plumbing shafts, `QUIET` rooms avoid street edges and the
/// core, `PUBLIC` rooms prefer the entrance half, `STREET` rooms prefer the
/// wall ring facing outside.
pub const TAG_WET: u8 = 1 << 0;
/// Quiet room tag bit (see [`TAG_WET`]).
pub const TAG_QUIET: u8 = 1 << 1;
/// Public room tag bit (see [`TAG_WET`]).
pub const TAG_PUBLIC: u8 = 1 << 2;
/// Street-facing room tag bit (see [`TAG_WET`]).
pub const TAG_STREET: u8 = 1 << 3;

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
    /// Minimum placements per floor (enforced after fill rolls).
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default)]
    pub min_count: u8,
    /// Adjacency/program tag bits (`TAG_*`); 0 = no preference.
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default)]
    pub tags: u8,
    /// Doors punched per room (`0` means 1). Serde-defaulted.
    #[serde(default)]
    pub doors: u8,
}

impl BlueprintRoom {
    /// Convenience constructor keeping call sites short.
    #[must_use]
    #[allow(clippy::too_many_arguments)] // one value per rule field, mirroring the record
    pub const fn new(
        kind: u8,
        weight: f32,
        min_w: u8,
        max_w: u8,
        min_d: u8,
        max_d: u8,
        min_count: u8,
        tags: u8,
        doors: u8,
    ) -> Self {
        Self {
            kind,
            weight,
            min_w,
            max_w,
            min_d,
            max_d,
            min_count,
            tags,
            doors,
        }
    }

    /// Doors this template punches per room (`0` reads as 1).
    #[must_use]
    pub const fn door_count(&self) -> u8 {
        if self.doors == 0 {
            1
        } else {
            self.doors
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
    /// Typical floors generate once and clone (`0`) vs re-roll per storey
    /// (nonzero). Serde-defaulted so pre-M13 files parse.
    #[serde(default)]
    pub vary_typical: u8,
    /// Circulation core wanders per floor (nonzero) vs stacks vertically
    /// (`0`, the default). Serde-defaulted so pre-M13 files parse.
    #[serde(default)]
    pub wandering_core: u8,
    /// Max apartment/office units per floor (`0` = open plan, no subdivision).
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default)]
    pub unit_max: u8,
    /// Plumbing shaft columns per building (`0` = off; capped at 4).
    /// WET-tagged rooms snap to these columns on every floor.
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default)]
    pub wet_shafts: u8,
    /// Ground-floor blueprint override as a `ZoneType` index
    /// (`255` = none): when set, floor 0 uses that zone's blueprint
    /// (mixed-use base, e.g. retail under housing).
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default = "default_ground_zone")]
    pub ground_zone: u8,
    /// Corridor policy: `0` = double-loaded fill (rooms both sides),
    /// `1` = single-loaded (rooms north of a south corridor band).
    /// Serde-defaulted so pre-M14 files parse.
    #[serde(default)]
    pub corridor: u8,
    /// Second-furniture-piece probability in percent (`0–100`): every room
    /// always stamps its primary piece when it fits; the secondary piece
    /// rolls against this density. Serde-defaulted so pre-M15 files parse.
    #[serde(default = "default_furn_density")]
    pub furn_density: u8,
}

/// Serde default for [`Blueprint::ground_zone`]: no override.
#[must_use]
pub const fn default_ground_zone() -> u8 {
    255
}

/// Serde default for [`Blueprint::furn_density`]: most rooms gain a second
/// piece.
#[must_use]
pub const fn default_furn_density() -> u8 {
    70
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
            BlueprintRoom::new(10, 3.0, 3, 6, 3, 6, 0, TAG_PUBLIC | TAG_STREET, 2), // lobby / lounge
            BlueprintRoom::new(11, 6.0, 3, 4, 3, 4, 0, TAG_PUBLIC, 1),              // open office
            BlueprintRoom::new(12, 4.0, 3, 5, 2, 4, 0, TAG_PUBLIC | TAG_QUIET, 1),  // meeting
            BlueprintRoom::new(13, 3.0, 2, 3, 2, 3, 0, TAG_WET, 1),                 // utility
        ],
        ZoneType::Residential => &[
            BlueprintRoom::new(20, 4.0, 3, 5, 3, 5, 0, TAG_PUBLIC | TAG_STREET, 2), // living
            BlueprintRoom::new(21, 3.0, 2, 3, 2, 3, 1, TAG_WET | TAG_PUBLIC, 1),    // kitchen
            BlueprintRoom::new(22, 4.0, 3, 4, 3, 4, 0, TAG_QUIET, 1),               // bedroom
            BlueprintRoom::new(23, 1.0, 1, 2, 1, 2, 1, TAG_WET | TAG_QUIET, 1),     // bathroom
        ],
        ZoneType::Commercial => &[
            BlueprintRoom::new(30, 3.0, 4, 6, 3, 5, 0, TAG_PUBLIC | TAG_STREET, 2), // retail floor
            BlueprintRoom::new(31, 3.0, 3, 5, 3, 5, 0, TAG_PUBLIC, 1),              // office/flex
            BlueprintRoom::new(32, 2.0, 2, 3, 2, 3, 0, TAG_WET, 1),                 // stockroom
        ],
        ZoneType::Industrial => &[
            BlueprintRoom::new(40, 5.0, 4, 7, 3, 6, 0, TAG_PUBLIC, 1), // open work bay
            BlueprintRoom::new(41, 2.0, 2, 3, 2, 3, 0, TAG_PUBLIC | TAG_QUIET, 1), // office/reception
            BlueprintRoom::new(42, 1.0, 1, 2, 1, 2, 0, TAG_WET, 1),                // washroom
        ],
        ZoneType::Park => &[BlueprintRoom::new(50, 1.0, 2, 3, 2, 3, 0, 0, 1)], // small shed
    };

    // Unit/shaft/corridor policy per zone: homes subdivide into apartments
    // with stacked plumbing and a retail-capable base; workplaces stay open
    // plan with one wet shaft; sheds stay simple.
    let (unit_max, wet_shafts, ground_zone, corridor, furn_density) = match zone {
        ZoneType::Downtown => (0, 1, 255, 0, 60),
        ZoneType::Residential => (3, 2, ZoneType::Commercial as u8, 0, 75),
        ZoneType::Commercial => (0, 1, 255, 0, 65),
        ZoneType::Industrial => (0, 1, 255, 0, 50),
        ZoneType::Park => (0, 0, 255, 0, 30),
    };

    // Copy the live rooms into the fixed array's prefix (the rest stay default).
    let mut rooms = [BlueprintRoom::default(); MAX_BLUEPRINT_ROOMS];
    rooms[..room_slice.len()].copy_from_slice(room_slice);

    Blueprint {
        margin,
        core_size,
        room_count: room_slice.len() as u8,
        rooms,
        vary_typical: 0,
        wandering_core: 0,
        unit_max,
        wet_shafts,
        ground_zone,
        corridor,
        furn_density,
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
/// meaningful only where the tile is `Room`; else 0). `furn` is a parallel
/// furniture layer (`FURN_*` codes, 0 where bare) stamped strictly inside
/// room rects, so tile semantics never change under renderers.
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
    /// Furniture code per tile (parallel to `tiles`, 0 where bare; only set
    /// where the tile is `Room`).
    pub furn: Vec<u8>,
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
            furn: vec![0; n],
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

    /// Read the furniture code at `(x, z)`, 0 when bare or out of bounds.
    #[must_use]
    pub fn furniture(&self, x: u8, z: u8) -> u8 {
        if x >= self.width || z >= self.depth {
            0
        } else {
            self.furn[self.index(x, z)]
        }
    }

    /// Wall cells that can host windows: exterior ring tiles on the `side`
    /// edge, excluding corners and doors.
    ///
    /// Renderers derive glazing from this instead of a wire tile: a window
    /// is a wall with daylight on one side and a room on the other. Pure
    /// geometry over the finished floor — deterministic, no hash.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::layout::{DoorSide, Floor, Tile};
    /// let mut f = Floor::empty(6, 6);
    /// for x in 0..6 {
    ///     for z in 0..6 {
    ///         if x == 0 || z == 0 || x == 5 || z == 5 {
    ///             f.tiles[z * 6 + x] = Tile::Wall;
    ///         }
    ///     }
    /// }
    /// // Four west-edge candidates; a door removes one.
    /// assert_eq!(Floor::window_cells(&f, DoorSide::West).len(), 4);
    /// f.tiles[2 * 6] = Tile::Door;
    /// assert_eq!(Floor::window_cells(&f, DoorSide::West).len(), 3);
    /// ```
    #[must_use]
    pub fn window_cells(floor: &Floor, side: DoorSide) -> Vec<(u8, u8)> {
        let (w, d) = (floor.width, floor.depth);
        if w < 3 || d < 3 {
            return Vec::new();
        }
        let edge: Vec<(u8, u8)> = match side {
            DoorSide::West => (1..d - 1).map(|z| (0, z)).collect(),
            DoorSide::East => (1..d - 1).map(|z| (w - 1, z)).collect(),
            DoorSide::North => (1..w - 1).map(|x| (x, 0)).collect(),
            DoorSide::South => (1..w - 1).map(|x| (x, d - 1)).collect(),
        };
        edge.into_iter()
            .filter(|(x, z)| floor.tiles[floor.index(*x, *z)] == Tile::Wall)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Furniture — per-room-kind fitting sets stamped inside room rects
// ---------------------------------------------------------------------------

/// No furniture (bare tile).
pub const FURN_NONE: u8 = 0;
/// Bed (bedrooms).
pub const FURN_BED: u8 = 1;
/// Table (living, meeting, work bays).
pub const FURN_TABLE: u8 = 2;
/// Counter run (kitchens, retail, washrooms).
pub const FURN_COUNTER: u8 = 3;
/// Desk (offices, lobbies, receptions).
pub const FURN_DESK: u8 = 4;
/// Shelf (stockrooms, living rooms, utility).
pub const FURN_SHELF: u8 = 5;
/// Bath fixture (bathrooms).
pub const FURN_BATH: u8 = 6;

/// Furniture set for a room kind: `(primary, secondary)` as
/// `(code, width, depth)` pieces. The primary stamps whenever it fits the
/// room rect (clamped); the secondary rolls against the blueprint's
/// `furn_density`. Unknown kinds get a shelf. A `(0, 0, 0)` secondary means
/// none.
#[must_use]
pub const fn furniture_set(kind: u8) -> [(u8, u8, u8); 2] {
    match kind {
        10 => [(FURN_DESK, 2, 1), (FURN_SHELF, 1, 1)], // lobby / lounge
        11 => [(FURN_DESK, 2, 1), (FURN_SHELF, 1, 1)], // open office
        12 => [(FURN_TABLE, 2, 2), (FURN_NONE, 0, 0)], // meeting
        13 => [(FURN_SHELF, 1, 1), (FURN_NONE, 0, 0)], // utility
        20 => [(FURN_TABLE, 2, 2), (FURN_SHELF, 1, 1)], // living
        21 => [(FURN_COUNTER, 2, 1), (FURN_TABLE, 1, 1)], // kitchen
        22 => [(FURN_BED, 2, 3), (FURN_SHELF, 1, 1)],  // bedroom
        23 => [(FURN_BATH, 1, 2), (FURN_NONE, 0, 0)],  // bathroom
        30 => [(FURN_COUNTER, 3, 1), (FURN_SHELF, 1, 1)], // retail floor
        31 => [(FURN_DESK, 2, 1), (FURN_NONE, 0, 0)],  // office/flex
        32 => [(FURN_SHELF, 2, 1), (FURN_NONE, 0, 0)], // stockroom
        40 => [(FURN_TABLE, 3, 2), (FURN_SHELF, 1, 1)], // open work bay
        41 => [(FURN_DESK, 2, 1), (FURN_NONE, 0, 0)],  // office/reception
        42 => [(FURN_COUNTER, 1, 1), (FURN_NONE, 0, 0)], // washroom
        50 => [(FURN_SHELF, 1, 1), (FURN_NONE, 0, 0)], // small shed
        _ => [(FURN_SHELF, 1, 1), (FURN_NONE, 0, 0)],  // custom kinds
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
            DoorSide::West,
            42,
            false,
            BuildingRole::Ordinary,
            ZoneType::Commercial,
        );
        assert_eq!(ctx.floor_count, 2);
        assert!(ctx.is_built());
    }

    #[test]
    fn context_floor_count_respects_cap() {
        // 120u at 4u/floor = 30 floors, capped to 8.
        let ctx = InteriorContext::new(
            1,
            ZoneType::Downtown,
            [0.0; 5],
            120.0,
            4.0,
            8,
            10,
            10,
            2,
            DoorSide::East,
            5,
            true,
            BuildingRole::Landmark,
            ZoneType::Commercial,
        );
        assert_eq!(ctx.floor_count, 8);
    }

    #[test]
    fn unbuilt_lot_has_zero_floors() {
        let ctx = InteriorContext::new(
            1,
            ZoneType::Park,
            [0.0; 5],
            0.0,
            4.0,
            64,
            4,
            4,
            0,
            DoorSide::South,
            5,
            false,
            BuildingRole::Ordinary,
            ZoneType::Residential,
        );
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
    fn frontage_depth_follows_entrance_edge() {
        // Depth is the lot extent perpendicular to the entrance: an 8x5 lot
        // entered from the west is 8 deep; from the north it is 5 deep.
        let west = InteriorContext::new(
            1,
            ZoneType::Commercial,
            [0.0; 5],
            12.0,
            4.0,
            64,
            8,
            5,
            1,
            DoorSide::West,
            9,
            false,
            BuildingRole::Ordinary,
            ZoneType::Residential,
        );
        assert_eq!(west.frontage_depth, 8);
        let north = InteriorContext::new(
            1,
            ZoneType::Commercial,
            [0.0; 5],
            12.0,
            4.0,
            64,
            8,
            5,
            1,
            DoorSide::North,
            9,
            false,
            BuildingRole::Ordinary,
            ZoneType::Residential,
        );
        assert_eq!(north.frontage_depth, 5);
    }

    #[test]
    fn floor_grid_indexing_is_row_major() {
        let f = Floor::empty(4, 3);
        assert_eq!(f.index(3, 2), 11); // 2*4 + 3
        assert_eq!(f.tile(0, 0), Tile::Void);
        assert_eq!(f.tile(9, 9), Tile::Void); // OOB clamps to Void
    }
}
