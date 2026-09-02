//! # data.rs
//!
//! Core data types exchanged by the Urbix engine.
//!
//! This module defines the flat, `repr(C)` records that travel across the FFI
//! boundary and form the binary chunk wire format. Keeping every on-the-wire
//! type `repr(C)` with explicit padding guarantees a stable, deterministic
//! memory layout regardless of compiler or platform (within matching ABI).
//!
//! ## Types (planned)
//!
//! - `ChunkId`        — integer key identifying a chunk by `(cx, cy)`.
//! - `InteriorId`     — deterministic id for a built lot's future interior.
//! - `CellFlags`      — bitflags (`is_street`, `is_park`, ...).
//! - `Cell`           — one city cell: height, zone affinity, palette, interior id.
//! - `ChunkHeader`    — header of a chunk buffer (coords, cell count, seed).
//! - `ChunkBuffer`    — owned wrapper around a chunk's packed data.
//!
//! ## Wire format
//!
//! A chunk is transmitted as `ChunkHeader` followed by `cell_count` packed
//! `Cell` records with no padding between them. See `Urbix_Project.md` §2.3
//! for the byte-level layout reference.

// TODO(Milestone 1): define the repr(C) data types.
