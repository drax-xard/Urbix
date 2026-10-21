//! # engine.rs
//!
//! Engine wrapper and Tauri command layer for the Urbix Explorer v2.
//!
//! Unlike the v1 explorer — which reached the engine through a dependency-free
//! C micro-server (`serve.c`) over HTTP — this app holds a single
//! [`WorldEngine`] behind a mutex and exposes it to the webview through Tauri
//! commands. Every command reads real Rust structs (never manual buffer
//! slicing), so the wire-format discipline that `serve.c` had to uphold does
//! not exist here.
//!
//! ## State
//!
//! One [`EngineState`] is created at startup and registered with Tauri. It
//! holds `Option<WorldEngine>` (present once `start_world` has run) plus the
//! active [`WorldConfig`], so `get_config` can answer before any world exists.
//! All commands lock the mutex for their duration; the engine is intentionally
//! single-user (it owns chunk + interior caches), and Tauri command handlers
//! are already serialized per window by the runtime.
//!
//! ## Coordinate convention
//!
//! Everything the frontend sees is in **world cells** (`1 cell = 4 m`), the
//! same convention the engine and the v1 viewer use. Vertical placement uses
//! the config's `interior_floor_height` so storeys line up with generation.

use std::sync::Mutex;

use serde::Serialize;
use tauri::State;

use urbix::config::{WorldConfig, DEFAULT_ZONE_HUES};
use urbix::data::CellFlags;
use urbix::engine::WorldEngine;
use urbix::layout::{DoorSide, Tile};
use urbix::zones::ZONE_COUNT;

/// Human-readable zone names, index-aligned with [`ZoneType`].
///
/// Kept here (rather than derived) so the menu labels match the SDK's
/// `ZONE_NAMES` table byte-for-byte and the frontend needs no translation.
pub const ZONE_NAMES: [&str; ZONE_COUNT] = [
    "downtown",
    "residential",
    "commercial",
    "industrial",
    "park",
];

/// Pedestrian eye height in world units (≈ 1.7 m at the 1 cell = 4 m canon).
pub const EYE_HEIGHT: f32 = 1.7;

/// Bound on the local walkable-cell scan around a candidate district site, in
/// cells. Enough to cross a site's own block lattice without scanning an
/// entire district (sites are thousands of cells apart, but the walkable
/// streets of a district hug its centre).
const DISTRICT_SCAN_RADIUS: i64 = 384;

/// Errors surfaced to the frontend as rejected promises.
#[derive(Debug, Serialize)]
pub struct CommandError {
    /// Human-readable failure reason.
    pub message: String,
}

impl CommandError {
    /// Build an error from anything displayable.
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl From<String> for CommandError {
    fn from(message: String) -> Self {
        Self { message }
    }
}

/// Result alias for the command layer.
type CmdResult<T> = Result<T, CommandError>;

/// Live engine state shared across Tauri commands.
#[derive(Default)]
pub struct EngineState {
    /// The world engine, present once `start_world` has successfully run.
    engine: Option<WorldEngine>,
    /// The config the engine was (or will be) built from. Always populated so
    /// `get_config` can answer menu queries before a world exists.
    config: WorldConfig,
}

/// Convenience newtype so `State<EngineState>` can be `Mutex`-wrapped.
pub type SharedEngine = Mutex<EngineState>;

// ---------------------------------------------------------------------------
// Frontend-facing data shapes (serde → JSON)
// ---------------------------------------------------------------------------

/// A single cell of a chunk, in the shape the frontend's mesh builder expects.
#[derive(Debug, Clone, Serialize)]
pub struct CellDto {
    /// World cell x.
    pub x: i32,
    /// World cell z.
    pub z: i32,
    /// Building height in world units (0 = street/open).
    pub h: f32,
    /// Dominant zone index (argmax of the affinity vector).
    pub zone: u8,
    /// Facade palette index.
    pub pal: u8,
    /// `CellFlags` bits (street/park/arterial/plaza/sidewalk/greenway).
    pub flags: u8,
    /// Deterministic interior key (0 = no interior).
    pub interior_id: u64,
    /// Exact interior key as a string for frontend grouping.
    ///
    /// `interior_id` is a `u64`, but JSON parses numbers through a JS
    /// `Number` (53-bit mantissa), which collapses nearby keys. Grouping
    /// meshes by lot needs the exact value, so this hex form is carried
    /// alongside the numeric compatibility field.
    pub lot_key: String,
}

/// A generated chunk: header fields plus its cells.
#[derive(Debug, Clone, Serialize)]
pub struct ChunkDto {
    /// Chunk column index.
    pub cx: i32,
    /// Chunk row index.
    pub cy: i32,
    /// Cells per side.
    pub chunk_size: u16,
    /// Row-major cell list (`chunk_size²` entries).
    pub cells: Vec<CellDto>,
}

/// One floor's grids, matching the v1 interior payload layout.
#[derive(Debug, Clone, Serialize)]
pub struct FloorDto {
    /// Grid width in tiles.
    pub w: u16,
    /// Grid depth in tiles.
    pub d: u16,
    /// Row-major tile kinds (`Tile` enum values).
    pub tiles: Vec<u8>,
    /// Row-major room-kind tags (0 where not a room).
    pub kinds: Vec<u8>,
    /// Row-major furniture codes (0 where bare).
    pub furn: Vec<u8>,
}

/// A building interior: per-storey grids plus metadata.
#[derive(Debug, Clone, Serialize)]
pub struct InteriorDto {
    /// Stable interior key.
    pub interior_id: u64,
    /// Dominant zone index.
    pub zone: u8,
    /// Door side index (0 west, 1 east, 2 north, 3 south).
    pub door_side: u8,
    /// Footprint width in tiles.
    pub footprint_w: u16,
    /// Footprint depth in tiles.
    pub footprint_d: u16,
    /// Number of storeys.
    pub floor_count: u16,
    /// World cell x of the queried lot (echoed for the frontend's origin math).
    pub wx: i32,
    /// World cell z of the queried lot.
    pub wz: i32,
    /// One entry per storey.
    pub floors: Vec<FloorDto>,
    /// Enumerated rooms across all storeys (same records as `get_rooms`).
    pub rooms: Vec<RoomDto>,
}

/// One enumerated room record.
#[derive(Debug, Clone, Serialize)]
pub struct RoomDto {
    /// Storey index.
    pub floor: u8,
    /// Left edge in grid cells.
    pub x: u8,
    /// Top edge in grid cells.
    pub z: u8,
    /// Width in cells.
    pub w: u8,
    /// Depth in cells.
    pub d: u8,
    /// Opaque room-kind tag.
    pub kind: u8,
    /// Apartment index (255 = outside every unit).
    pub unit: u8,
    /// Floor area in tiles.
    pub area: u16,
}

/// Static world/menu configuration snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigDto {
    /// Engine crate version string.
    pub sdk: String,
    /// Active seed.
    pub seed: u64,
    /// Cells per chunk side.
    pub chunk_size: u16,
    /// Chunk Chebyshev draw radius.
    pub draw_distance: u32,
    /// World units per storey.
    pub floor_height: f32,
    /// Metres per cell (scale canon).
    pub cell_meters: f32,
    /// Pedestrian eye height in world units.
    pub eye_height: f32,
    /// Per-zone RGB hues.
    pub zone_hues: [[u8; 3]; ZONE_COUNT],
    /// Per-zone display names.
    pub zone_names: [&'static str; ZONE_COUNT],
}

/// A resolved spawn position at ground level on a walkable cell.
#[derive(Debug, Clone, Serialize)]
pub struct SpawnDto {
    /// World cell x of the spawn.
    pub wx: i32,
    /// World cell z of the spawn.
    pub wz: i32,
    /// World-space x (cell centre) for the camera.
    pub x: f32,
    /// World-space z (cell centre) for the camera.
    pub z: f32,
    /// World-space eye y for the camera.
    pub y: f32,
    /// Dominant zone index of the chosen cell.
    pub zone: u8,
    /// Chebyshev distance from the requested origin, in cells.
    pub distance: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Whether a cell is walkable ground (any non-built paved/green surface).
///
/// Takes the raw `CellFlags` bits so it works uniformly on engine cells and on
/// our own [`CellDto`]s (which carry the bits as a `u8`).
fn is_walkable_ground(bits: u8) -> bool {
    const WALKABLE: u8 = CellFlags::IS_STREET.bits()
        | CellFlags::IS_SIDEWALK.bits()
        | CellFlags::IS_PLAZA.bits()
        | CellFlags::IS_GREENWAY.bits()
        | CellFlags::IS_PARK.bits();
    bits & WALKABLE != 0
}

/// Index of the dominant zone (argmax affinity) for a cell.
fn dominant_zone(affinity: &[f32; ZONE_COUNT]) -> u8 {
    let mut best = 0usize;
    for (i, w) in affinity.iter().enumerate() {
        if *w > affinity[best] {
            best = i;
        }
    }
    best as u8
}

/// Convert a `Tile` to its wire byte.
fn tile_byte(t: Tile) -> u8 {
    t as u8
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Return the static configuration snapshot for the menu.
///
/// Works before a world is started (the state carries a default config), so
/// the zone dropdown can be populated immediately on load.
#[tauri::command]
pub fn get_config(state: State<'_, SharedEngine>) -> CmdResult<ConfigDto> {
    let st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    Ok(config_dto(&st.config))
}

/// Build a config snapshot from a `WorldConfig`.
fn config_dto(cfg: &WorldConfig) -> ConfigDto {
    ConfigDto {
        sdk: urbix::VERSION.to_string(),
        seed: cfg.seed,
        chunk_size: cfg.chunk_size,
        draw_distance: cfg.draw_distance,
        floor_height: cfg.interior_floor_height,
        cell_meters: 4.0,
        eye_height: EYE_HEIGHT,
        zone_hues: cfg.zone_hues,
        zone_names: ZONE_NAMES,
    }
}

/// Create (or re-create) the world for `seed` with the default config.
///
/// `seed` arrives as a string so the full `u64` range survives the JSON IPC
/// boundary (a JS number cannot represent integers above 2^53 exactly).
/// Re-seeding rebuilds the Voronoi map and clears the chunk/interior caches,
/// so a subsequent `get_chunks` never mixes two worlds.
#[tauri::command]
pub fn start_world(state: State<'_, SharedEngine>, seed: String) -> CmdResult<ConfigDto> {
    let seed: u64 = seed
        .trim()
        .parse()
        .map_err(|_| CommandError::new(format!("invalid seed '{seed}'")))?;
    let mut st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    let cfg = WorldConfig { seed, ..st.config };
    let engine = WorldEngine::with_config(cfg);
    st.config = cfg;
    st.engine = Some(engine);
    Ok(config_dto(&st.config))
}

/// Find the nearest walkable cell of the requested zone to `(wx, wz)`.
///
/// Districts are Voronoi cells whose sites are spread over
/// `[-voronoi_span, voronoi_span]²` (default ±10 000 cells), so zones are
/// thousands of cells apart — a cell-by-cell scan from the origin is hopeless.
/// Instead this uses the diagram directly:
///
/// 1. Take the diagram's sites, keep those tagged with the requested zone.
/// 2. Order them by distance to `(wx, wz)`.
/// 3. For each candidate site (nearest first), scan a bounded ring around it
///    for a *walkable* cell whose dominant affinity is the requested zone.
/// 4. Return globally-nearest spawn found; the first site that yields one is
///    the closest target district, so it wins.
///
/// The "closest region with the desired zone type" is therefore the nearest
/// matching district, and the spawn within it is the cell closest to that
/// district's own site — a legible, street-level placement.
#[tauri::command]
pub fn find_zone_start(
    state: State<'_, SharedEngine>,
    zone: u8,
    wx: i32,
    wz: i32,
) -> CmdResult<SpawnDto> {
    let mut st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    resolve_zone_spawn(&mut st, zone, wx, wz)
}

/// Core of [`find_zone_start`], split out so tests can drive it directly.
fn resolve_zone_spawn(st: &mut EngineState, zone: u8, wx: i32, wz: i32) -> CmdResult<SpawnDto> {
    if zone as usize >= ZONE_COUNT {
        return Err(CommandError::new(format!("zone {zone} out of range")));
    }
    // Read the config-derived scalars before borrowing the engine mutably.
    let n = i64::from(st.config.chunk_size);
    let floor_height = st.config.interior_floor_height.max(1e-3);
    let ox = i64::from(wx);
    let oz = i64::from(wz);

    let engine = st
        .engine
        .as_mut()
        .ok_or_else(|| CommandError::new("world not started"))?;

    // Candidate sites of the requested zone, nearest to the origin first.
    let mut sites: Vec<(f64, i64, i64)> = engine
        .voronoi()
        .sites()
        .iter()
        .filter(|s| s.zone as u8 == zone)
        .map(|s| {
            let sx = s.x.round() as i64;
            let sz = s.y.round() as i64;
            let dx = (sx - ox) as f64;
            let dz = (sz - oz) as f64;
            (dx * dx + dz * dz, sx, sz)
        })
        .collect();
    if sites.is_empty() {
        return Err(CommandError::new(format!(
            "no {} district exists in this world",
            ZONE_NAMES[zone as usize]
        )));
    }
    sites.sort_by(|a, b| a.0.total_cmp(&b.0));

    let mut chunk_cache: std::collections::HashMap<(i32, i32), Vec<CellDto>> =
        std::collections::HashMap::new();

    // The nearest matching site that yields a walkable cell wins; earlier
    // sites were farther from the origin, so no better district can exist.
    for (_sd, sx, sz) in sites {
        if let Some(spawn) = scan_disk_for_zone(
            engine,
            &mut chunk_cache,
            &ScanParams {
                n,
                floor_height,
                zone,
                site_x: sx,
                site_z: sz,
                radius: DISTRICT_SCAN_RADIUS,
            },
        ) {
            return Ok(spawn);
        }
    }

    Err(CommandError::new(format!(
        "no walkable {} cell found near any such district",
        ZONE_NAMES[zone as usize]
    )))
}

/// Inputs to [`scan_disk_for_zone`], grouped to keep the signature legible.
struct ScanParams {
    /// Cells per chunk side.
    n: i64,
    /// World units per storey (used for the spawn eye height).
    floor_height: f32,
    /// Zone index to match.
    zone: u8,
    /// Disk centre, world cell x.
    site_x: i64,
    /// Disk centre, world cell z.
    site_z: i64,
    /// Maximum Chebyshev-ish scan radius in cells.
    radius: i64,
}

/// Scan a disk of radius `params.radius` around `(site_x, site_z)` for a
/// walkable cell whose dominant affinity is `params.zone` and which is
/// unbuilt. Returns the cell nearest the centre, converted to a spawn, or
/// `None` if the disk holds none.
fn scan_disk_for_zone(
    engine: &mut WorldEngine,
    chunk_cache: &mut std::collections::HashMap<(i32, i32), Vec<CellDto>>,
    params: &ScanParams,
) -> Option<SpawnDto> {
    let ScanParams {
        n,
        floor_height,
        zone,
        site_x: cx,
        site_z: cz,
        radius,
    } = *params;
    for r in 0..=radius {
        let mut best: Option<(f64, i64, i64, u8)> = None;
        for (dx, dz) in ring_cells(r) {
            let x = cx + dx;
            let z = cz + dz;
            let ch_cx = x.div_euclid(n) as i32;
            let ch_cy = z.div_euclid(n) as i32;
            let lx = x.rem_euclid(n) as usize;
            let ly = z.rem_euclid(n) as usize;
            let cells = chunk_cache.entry((ch_cx, ch_cy)).or_insert_with(|| {
                let buf = engine.generate_chunk(ch_cx, ch_cy);
                chunk_to_cells(&buf, ch_cx, ch_cy)
            });
            let Some(cell) = cells.get(ly * n as usize + lx) else {
                continue;
            };
            if cell.h > 0.0 || !is_walkable_ground(cell.flags) || cell.zone != zone {
                continue;
            }
            let d2 = (dx * dx + dz * dz) as f64;
            if best.is_none_or(|(bd, _, _, _)| d2 < bd) {
                best = Some((d2, x, z, cell.zone));
            }
        }
        if let Some((_d2, bx, bz, dom)) = best {
            return Some(SpawnDto {
                wx: bx as i32,
                wz: bz as i32,
                x: bx as f32 + 0.5,
                z: bz as f32 + 0.5,
                y: EYE_HEIGHT.min(floor_height),
                zone: dom,
                distance: r,
            });
        }
    }
    None
}

/// The cells of Chebyshev ring `radius` around the origin, in scan order.
///
/// Radius 0 yields only the origin. Larger rings walk the perimeter (no
/// allocation beyond the returned Vec; rings are small for typical radii).
fn ring_cells(radius: i64) -> Vec<(i64, i64)> {
    if radius == 0 {
        return vec![(0, 0)];
    }
    let mut out = Vec::with_capacity((8 * radius) as usize);
    for x in -radius..=radius {
        out.push((x, -radius));
        out.push((x, radius));
    }
    for z in (-radius + 1)..radius {
        out.push((-radius, z));
        out.push((radius, z));
    }
    out
}

/// Convert a chunk buffer's cells to DTOs with absolute world coordinates.
fn chunk_to_cells(buf: &urbix::data::ChunkBuffer, cx: i32, cy: i32) -> Vec<CellDto> {
    let header = buf.header();
    let n = usize::from(header.chunk_size);
    let mut out = Vec::with_capacity(buf.cell_count());
    for i in 0..buf.cell_count() {
        let cell = buf.get_cell(i);
        let lx = (i % n) as i32;
        let ly = (i / n) as i32;
        out.push(CellDto {
            x: cx * header.chunk_size as i32 + lx,
            z: cy * header.chunk_size as i32 + ly,
            h: cell.height,
            zone: dominant_zone(&cell.zone_affinity),
            pal: cell.palette_id,
            flags: cell.flags.bits(),
            interior_id: cell.interior_id,
            lot_key: if cell.interior_id == 0 {
                String::new()
            } else {
                format!("{:016x}", cell.interior_id)
            },
        });
    }
    out
}

/// Maximum chunks accepted by one `get_chunks` call.
///
/// The frontend streams a 7×7 square (49 chunks) on first load and then asks
/// only for the newly exposed edge, so 64 leaves headroom without letting a
/// bad request balloon the IPC payload.
const MAX_CHUNK_BATCH: usize = 64;

/// Build DTOs for the requested chunk coordinates, skipping duplicates.
fn generate_chunks(engine: &mut WorldEngine, coords: &[[i32; 2]]) -> Vec<ChunkDto> {
    let mut seen = std::collections::HashSet::with_capacity(coords.len());
    let mut out = Vec::with_capacity(coords.len());
    for &[ccx, ccy] in coords {
        if !seen.insert((ccx, ccy)) {
            continue;
        }
        let buf = engine.generate_chunk(ccx, ccy);
        out.push(ChunkDto {
            cx: ccx,
            cy: ccy,
            chunk_size: buf.header().chunk_size,
            cells: chunk_to_cells(&buf, ccx, ccy),
        });
    }
    out
}

/// Generate (or fetch cached) an explicit list of chunks.
///
/// The frontend tracks which chunks it already holds and requests only the
/// new ones, so crossing a chunk border transfers roughly one edge instead of
/// reserializing the whole streaming square.
#[tauri::command]
pub fn get_chunks(
    state: State<'_, SharedEngine>,
    coords: Vec<[i32; 2]>,
) -> CmdResult<Vec<ChunkDto>> {
    if coords.len() > MAX_CHUNK_BATCH {
        return Err(CommandError::new(format!(
            "chunk batch too large ({} > {MAX_CHUNK_BATCH})",
            coords.len()
        )));
    }
    let mut st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    let engine = st
        .engine
        .as_mut()
        .ok_or_else(|| CommandError::new("world not started"))?;
    Ok(generate_chunks(engine, &coords))
}

/// Enumerate every room of a layout (ground blueprint resolved once).
fn collect_rooms(engine: &WorldEngine, layout: &urbix::layout::InteriorLayout) -> Vec<RoomDto> {
    let cfg = *engine.config();
    let main_bp = cfg.blueprint_for(layout.context.zone);
    let ground = urbix::interior::resolve_ground_override(
        &layout.context,
        &main_bp,
        &cfg.interior_blueprints,
    );

    let mut out = Vec::new();
    for (f, floor) in layout.floors.iter().enumerate() {
        let bp = if f == 0 { ground } else { &main_bp };
        let units = urbix::interior::unit_rects_for_floor(
            layout.id,
            &layout.context,
            f as u8,
            bp,
            &main_bp,
        );
        for r in urbix::interior::rooms_of_floor(floor, f as u8, &units) {
            out.push(RoomDto {
                floor: r.floor,
                x: r.x,
                z: r.z,
                w: r.w,
                d: r.d,
                kind: r.kind,
                unit: r.unit,
                area: r.area,
            });
        }
    }
    out
}

/// Build a full interior DTO (grids + rooms) for the cell at `(wx, wz)`.
///
/// Returns `floor_count == 0` (empty `floors`/`rooms`) for unbuilt or sealed
/// cells, mirroring the v1 HTTP endpoint so the frontend's cross-fade logic
/// is unchanged.
fn interior_dto(engine: &mut WorldEngine, wx: i32, wz: i32) -> InteriorDto {
    let empty = InteriorDto {
        interior_id: 0,
        zone: 0,
        door_side: 0,
        footprint_w: 0,
        footprint_d: 0,
        floor_count: 0,
        wx,
        wz,
        floors: Vec::new(),
        rooms: Vec::new(),
    };

    let Some(layout) = engine.interior_layout(i64::from(wx), i64::from(wz)) else {
        return empty;
    };

    let floors: Vec<FloorDto> = layout
        .floors
        .iter()
        .map(|f| FloorDto {
            w: u16::from(f.width),
            d: u16::from(f.depth),
            tiles: f.tiles.iter().map(|t| tile_byte(*t)).collect(),
            kinds: f.kinds.clone(),
            furn: f.furn.clone(),
        })
        .collect();

    let (footprint_w, footprint_d) = layout
        .floors
        .first()
        .map_or((0, 0), |f| (u16::from(f.width), u16::from(f.depth)));

    InteriorDto {
        interior_id: layout.id,
        zone: layout.context.zone as u8,
        door_side: layout.context.door_side as u8,
        footprint_w,
        footprint_d,
        floor_count: layout.floors.len() as u16,
        wx,
        wz,
        rooms: collect_rooms(engine, &layout),
        floors,
    }
}

/// Generate the interior (grids + rooms) for the built cell at `(wx, wz)`.
#[tauri::command]
pub fn get_interior(state: State<'_, SharedEngine>, wx: i32, wz: i32) -> CmdResult<InteriorDto> {
    let mut st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    let engine = st
        .engine
        .as_mut()
        .ok_or_else(|| CommandError::new("world not started"))?;
    Ok(interior_dto(engine, wx, wz))
}

/// Enumerate every room of the built cell at world `(wx, wz)`.
#[tauri::command]
pub fn get_rooms(state: State<'_, SharedEngine>, wx: i32, wz: i32) -> CmdResult<Vec<RoomDto>> {
    let mut st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    let engine = st
        .engine
        .as_mut()
        .ok_or_else(|| CommandError::new("world not started"))?;

    let Some(layout) = engine.interior_layout(i64::from(wx), i64::from(wz)) else {
        return Ok(Vec::new());
    };
    Ok(collect_rooms(engine, &layout))
}

/// Query the raw (continuous) zone-affinity weights at world `(wx, wz)`.
#[tauri::command]
pub fn get_zone(state: State<'_, SharedEngine>, wx: f64, wz: f64) -> CmdResult<[f32; ZONE_COUNT]> {
    let st = state
        .lock()
        .map_err(|_| CommandError::new("engine state poisoned"))?;
    let engine = st
        .engine
        .as_ref()
        .ok_or_else(|| CommandError::new("world not started"))?;
    Ok(engine.get_zone_affinity(wx, wz))
}

/// Zone-hue export used by tests; kept so default hues are never hardcoded twice.
#[allow(dead_code)]
pub const fn default_zone_hues() -> [[u8; 3]; ZONE_COUNT] {
    DEFAULT_ZONE_HUES
}

/// Encode a `DoorSide` for tests.
#[allow(dead_code)]
pub const fn door_side_index(side: DoorSide) -> u8 {
    side as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn fresh_state(seed: u64) -> SharedEngine {
        let mut st = EngineState::default();
        let cfg = WorldConfig { seed, ..st.config };
        st.config = cfg;
        st.engine = Some(WorldEngine::with_config(cfg));
        Mutex::new(st)
    }

    fn find(state: &SharedEngine, zone: u8, wx: i32, wz: i32) -> SpawnDto {
        let mut st = state.lock().unwrap();
        resolve_zone_spawn(&mut st, zone, wx, wz).expect("spawn resolves")
    }

    #[test]
    fn ring_cells_has_expected_size() {
        assert_eq!(ring_cells(0), vec![(0, 0)]);
        assert_eq!(ring_cells(1).len(), 8);
        assert_eq!(ring_cells(2).len(), 16);
    }

    #[test]
    fn dominant_zone_picks_argmax() {
        assert_eq!(dominant_zone(&[0.1, 0.6, 0.1, 0.1, 0.1]), 1);
        assert_eq!(dominant_zone(&[0.5, 0.2, 0.1, 0.1, 0.1]), 0);
    }

    #[test]
    fn walkable_ground_covers_streets_and_parks() {
        assert!(is_walkable_ground(CellFlags::IS_STREET.bits()));
        assert!(is_walkable_ground(CellFlags::IS_PARK.bits()));
        assert!(is_walkable_ground(CellFlags::IS_SIDEWALK.bits()));
        assert!(!is_walkable_ground(CellFlags::NONE.bits()));
    }

    #[test]
    fn find_zone_start_resolves_every_zone_across_seeds() {
        // Every zone is reachable near a few seeds; the spawn is walkable and
        // dominant-zone-matched by construction, so just assert it resolves.
        for seed in [445566u64, 1, 69420, 999] {
            let state = fresh_state(seed);
            for zone in 0..ZONE_COUNT as u8 {
                let spawn = find(&state, zone, 0, 0);
                assert_eq!(spawn.zone, zone, "seed {seed} zone {zone}");
            }
        }
    }

    #[test]
    fn chunk_to_cells_uses_absolute_coords() {
        let mut engine = WorldEngine::new(445566);
        let buf = engine.generate_chunk(-1, 2);
        let cells = chunk_to_cells(&buf, -1, 2);
        let n = usize::from(buf.header().chunk_size);
        assert_eq!(cells.len(), n * n);
        assert_eq!(cells[0].x, -(n as i32));
        assert_eq!(cells[0].z, 2 * n as i32);
        assert_eq!(cells[n].x, -(n as i32));
        assert_eq!(cells[n].z, 2 * n as i32 + 1);
        for cell in &cells {
            if cell.interior_id != 0 {
                assert_eq!(cell.lot_key, format!("{:016x}", cell.interior_id));
            }
        }
    }

    #[test]
    fn get_config_before_start_uses_defaults() {
        let st = EngineState::default();
        let dto = config_dto(&st.config);
        assert_eq!(dto.zone_names.len(), ZONE_COUNT);
        assert!(dto.floor_height > 0.0);
    }

    #[test]
    fn default_hues_match_config_constant() {
        assert_eq!(default_zone_hues(), DEFAULT_ZONE_HUES);
    }

    #[test]
    fn generate_chunks_dedupes_and_preserves_order() {
        let mut engine = WorldEngine::new(445566);
        let out = generate_chunks(&mut engine, &[[-1, 2], [0, 0], [-1, 2]]);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].cx, out[0].cy), (-1, 2));
        assert_eq!((out[1].cx, out[1].cy), (0, 0));
        let n = usize::from(out[0].chunk_size);
        assert_eq!(out[0].cells.len(), n * n);
    }

    #[test]
    fn interior_dto_carries_floors_and_rooms() {
        let mut engine = WorldEngine::new(445566);
        let n = i32::from(engine.config().chunk_size);
        let mut found = None;
        'outer: for cy in -1..=1 {
            for cx in -1..=1 {
                let buf = engine.generate_chunk(cx, cy);
                for i in 0..buf.cell_count() {
                    let cell = buf.get_cell(i);
                    if cell.height > 0.0 && cell.interior_id != 0 {
                        let lx = (i % n as usize) as i32;
                        let ly = (i / n as usize) as i32;
                        found = Some((cx * n + lx, cy * n + ly));
                        break 'outer;
                    }
                }
            }
        }
        let (wx, wz) = found.expect("a built cell near the origin");
        let dto = interior_dto(&mut engine, wx, wz);
        assert!(dto.floor_count > 0);
        assert_eq!(dto.floors.len(), usize::from(dto.floor_count));
        assert!(!dto.rooms.is_empty());
        assert!(dto.rooms.iter().all(|r| r.floor < dto.floor_count as u8));
    }

    #[test]
    fn interior_dto_is_empty_for_unbuilt_cells() {
        let mut engine = WorldEngine::new(445566);
        let buf = engine.generate_chunk(0, 0);
        let n = buf.header().chunk_size as usize;
        let (wx, wz) = (0..buf.cell_count())
            .find(|&i| buf.get_cell(i).height <= 0.0)
            .map(|i| ((i % n) as i32, (i / n) as i32))
            .expect("a street cell");
        let dto = interior_dto(&mut engine, wx, wz);
        assert_eq!(dto.floor_count, 0);
        assert!(dto.floors.is_empty());
        assert!(dto.rooms.is_empty());
    }
}
