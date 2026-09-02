//! # street.rs
//!
//! Street grid and block subdivision for the Urbix engine.
//!
//! This module decides which cells are part of the road network and how the
//! remaining space is divided into building blocks. Street layout is tuned per
//! zone: tight small blocks downtown, roomy blocks in residential areas, and
//! large wide blocks in industrial districts.
//!
//! ## Responsibilities
//!
//! - Given a cell's zone parameters, decide whether it is a street.
//! - Streets carry `height = 0` and are excluded from building placement.
//! - Delimits the interior of each block, which is then filled with a building
//!   footprint (or left open/park cells) by `building.rs`.
//!
//! ## Determinism
//!
//! Like everything else, the layout is derived deterministically from the cell
//! coordinates, the seed, and the zone parameters, so blocks are stable across
//! regeneration and consistent across chunk boundaries.

// TODO(Milestone 3): implement layout_block.
