//! # config.rs
//!
//! Global tunable parameters for the Urbix engine.
//!
//! This module centralizes every knob that shapes generation, so behaviour is
//! predictable and easy to experiment with. Configuration is captured in a
//! plain data struct (`WorldConfig`) rather than scattered constants.
//!
//! ## Fields
//!
//! - `seed`               — world seed; the same seed always yields the same city.
//! - `chunk_size`         — cells per chunk side (default 32; 16/64/128 supported).
//! - `draw_distance`      — chunk Chebyshev radius kept in cache before eviction.
//! - `voronoi_site_count` — number of district sites (24–48) spread over the span.
//!
//! Being `#[repr(C)]` and plain data, `WorldConfig` can cross the FFI boundary
//! so foreign consumers can configure the engine uniformly. `Default` provides
//! a sensible starting point for each knob.

/// A `#[repr(C)]` snapshot of every tunable that shapes world generation.
///
/// The struct holds only plain integer/float data so it is trivially
/// FFI-safe and can be passed by pointer to and from C. Construct it with
/// [`WorldConfig::default`] and override the fields you care about, or build
/// a fresh one via `..Default::default()`.
///
/// ## Example
///
/// ```
/// use urbix::config::WorldConfig;
///
/// let cfg = WorldConfig {
///     seed: 445566,
///     ..Default::default()
/// };
/// assert_eq!(cfg.seed, 445566);
/// assert!(cfg.is_valid());
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct WorldConfig {
    /// World seed; the same seed always yields the same city.
    pub seed: u64,
    /// Cells per chunk side. Must be > 0; defaults to 32.
    pub chunk_size: u16,
    /// Chunk Chebyshev radius kept in cache before eviction. Must be > 0.
    pub draw_distance: u32,
    /// Number of district sites in the Voronoi map. Typically 24–48.
    pub voronoi_site_count: u16,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            seed: 0,
            chunk_size: 32,
            draw_distance: 8,
            voronoi_site_count: 32,
        }
    }
}

impl WorldConfig {
    /// Validate the invariant-satisfying constraints on every field.
    ///
    /// Returns `false` if any field is out of its supported range, so
    /// callers (engine construction, FFI setters) can reject bad config early.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::config::WorldConfig;
    ///
    /// assert!(WorldConfig::default().is_valid());
    /// let bad = WorldConfig { chunk_size: 0, ..Default::default() };
    /// assert!(!bad.is_valid());
    /// ```
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.chunk_size > 0
            && self.draw_distance > 0
            && (16..=64).contains(&self.voronoi_site_count)
    }
}

#[cfg(test)]
mod tests {
    use super::WorldConfig;

    #[test]
    fn default_is_valid() {
        let cfg = WorldConfig::default();
        assert_eq!(cfg.chunk_size, 32);
        assert_eq!(cfg.voronoi_site_count, 32);
        assert!(cfg.is_valid());
    }

    #[test]
    fn rejects_bad_chunk_size() {
        let cfg = WorldConfig {
            chunk_size: 0,
            ..Default::default()
        };
        assert!(!cfg.is_valid());
    }

    #[test]
    fn rejects_bad_site_count() {
        // Site counts outside the supported 16..=64 band are invalid.
        let too_low = WorldConfig {
            voronoi_site_count: 4,
            ..Default::default()
        };
        let too_high = WorldConfig {
            voronoi_site_count: 128,
            ..Default::default()
        };
        let ok = WorldConfig {
            voronoi_site_count: 48,
            ..Default::default()
        };
        assert!(!too_low.is_valid());
        assert!(!too_high.is_valid());
        assert!(ok.is_valid());
    }

    #[test]
    fn supports_other_chunk_sizes() {
        for size in [16u16, 32, 64, 128] {
            assert!(WorldConfig {
                chunk_size: size,
                ..Default::default()
            }
            .is_valid());
        }
    }
}
