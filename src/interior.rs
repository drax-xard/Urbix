//! # interior.rs
//!
//! Interior id computation and the hook surface for future interior rooms.
//!
//! Every built lot (each building/home/factory footprint) is a candidate for a
//! future interactable interior. To make that evolution cheap and seamless,
//! the world model computes a stable `InteriorId` for every built cell today,
//! even though interiors are not yet generated or rendered.
//!
//! ## InteriorId
//!
//! `InteriorId = hash(cell_coords, seed, zone)` is deterministic and stable,
//! so a given doorway always leads to the *same* room across visits and across
//! runs with the same seed.
//!
//! ## Hook surface
//!
//! - `InteriorState` — what an interior run needs (room layout, size, fog,
//!   palette, exits).
//! - `enter(lot, player)` / `exit(lot, player)` — teleport into/out of a
//!   deterministic interior.
//! - `generate_interior(id) -> InteriorState` — stub returning a placeholder
//!   so the *interface* is wired and callable end-to-end.
//!
//! Until implemented, the stub returns a null/generic `InteriorState` and the
//! enter action is a no-op (or clearly logged). An interior is a *separate
//! mini-world* (its own small grid, not part of the infinite chunk city),
//! keyed by `InteriorId` and cached independently.

// TODO(Milestone 6): define InteriorState trait and stub generate_interior.
