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
//! Each chunk is generated from `hash(cx, cy, seed)` with no cross-chunk write
//! dependency. Adjacent chunks agree at their shared edges because every cell
//! queries the same continuous Voronoi field rather than a per-chunk local
//! state.

// TODO(Milestone 3): implement generate_chunk.
