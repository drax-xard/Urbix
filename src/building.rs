//! # building.rs
//!
//! Building footprint and appearance generation for the Urbix city engine.
//!
//! This module is the last step of per-cell construction. Given a cell that
//! is *not* a street (see `street.rs`), it assigns the building's height and
//! facade palette id, both derived deterministically from the cell's
//! coordinates and the world seed.
//!
//! ## Responsibilities
//!
//! - Derive a building's **height** via `hash` of the cell coordinates,
//!   clamped into the owning zone's `[height_min, height_max]` range so every
//!   built cell looks a little different while respecting its district.
//! - Select a **palette id** from the zone's facade palette, again from the
//!   hash, so neighbouring buildings vary but remain cohesive.
//! - Leave park/open cells and streets untouched (they carry `height = 0`).
//!
//! ## Determinism
//!
//! The same `(cell_x, cell_y, seed, zone)` always yields the same height and
//! palette. This is what keeps adjacent chunks consistent and the world
//! reproducible.

// TODO(Milestone 3): implement assign_building.
