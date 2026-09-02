//! # region.rs
//!
//! Voronoi region layer for the Urbix city engine.
//!
//! This module defines the *big, smooth* layer of generation: which part of
//! the world is downtown, residential, commercial, industrial, or park. It
//! builds a Voronoi diagram of a fixed set of sites from the seed and answers
//! fuzzy "zone affinity" queries at any world coordinate.
//!
//! ## Fuzzy borders
//!
//! Instead of hard edges, each query blends contributions from the *nearest
//! two* sites using a weighted distance ratio (smoothstep on the ratio of the
//! nearest to second-nearest distance). The result is a per-point zone-affinity
//! vector across a small palette of parameters (density, height range, block
//! style), yielding gradual, seamless transitions between districts.
//!
//! ## Longevity
//!
//! The Voronoi map is tiny (24–48 sites) and immutable; it is computed once at
//! engine construction and lives for the whole run. Zone queries therefore
//! stay cheap, and neighbouring chunks remain consistent because they query
//! the same continuous field.

// TODO(Milestone 2): implement VoronoiDiagram + zone query.
