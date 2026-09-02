//! # data.rs
//!
//! Core data types exchanged by the Urbix engine.
//!
//! This module defines the flat, `#[repr(C)]` records that travel across the
//! FFI boundary and form the binary chunk wire format. Keeping every
//! on-the-wire type `#[repr(C)]` with explicit padding guarantees a stable,
//! deterministic memory layout regardless of compiler or platform (within a
//! matching ABI), matching the C reference in `Urbix_Project.md` §2.3.
//!
//! ## Types
//!
//! - `ChunkId`        — integer key identifying a chunk by `(cx, cy)`.
//! - `InteriorId`     — deterministic id for a built lot's future interior.
//! - `CellFlags`      — bitflags (`IS_STREET`, `IS_PARK`, ...).
//! - `Cell`           — one city cell: height, zone affinity, palette, interior id.
//! - `ChunkHeader`    — header of a chunk buffer (coords, cell count, seed).
//! - `ChunkBuffer`    — owned wrapper around a chunk's packed data.
//!
//! ## Wire format
//!
//! A chunk is transmitted as `ChunkHeader` followed by `cell_count` packed
//! `Cell` records with no padding between them. `ChunkBuffer` owns that packed
//! byte stream while providing typed access to the header and cells.

use std::mem::{align_of, size_of};

/// Number of zone-affinity weights stored per cell. Must match
/// [`crate::zones::ZONE_COUNT`].
pub const ZONE_COUNT: usize = 5;

/// Identifier addressing a chunk by its integer `(cx, cy)` grid coordinates.
///
/// `#[repr(C)]` and hashable so it can serve as both an FFI-visible record and
/// an LRU-cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(C)]
pub struct ChunkId {
    /// Chunk column index.
    pub cx: i32,
    /// Chunk row index.
    pub cy: i32,
}

impl ChunkId {
    /// Build a new chunk id from grid coordinates.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::data::ChunkId;
    /// let id = ChunkId::new(-3, 5);
    /// assert_eq!((id.cx, id.cy), (-3, 5));
    /// ```
    #[must_use]
    pub const fn new(cx: i32, cy: i32) -> Self {
        Self { cx, cy }
    }
}

/// Deterministic id of a built lot's future interior.
///
/// Derived from the cell's coordinates, seed, and zone via [`crate::hash`], so
/// a given doorway always leads to the same room across visits. `0` means
/// "no interior" (street, park, or open cell).
pub type InteriorId = u64;

/// Bit flags describing per-cell traits.
///
/// Implemented as a small manual bitfield to avoid extra dependencies while
/// staying `#[repr(C)]`-friendly. A `CellFlags` value is opaque bits; use the
/// `IS_*` constants and the `contains`/setters to read and write it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub struct CellFlags(u8);

impl CellFlags {
    /// Empty set of flags.
    pub const NONE: CellFlags = CellFlags(0);
    /// The cell is a street (height is 0 and no building is placed).
    pub const IS_STREET: CellFlags = CellFlags(1 << 0);
    /// The cell is park/green (foliage or open ground).
    pub const IS_PARK: CellFlags = CellFlags(1 << 1);

    /// Whether all flags in `rhs` are present.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::data::CellFlags;
    /// assert!(CellFlags::IS_STREET.contains(CellFlags::IS_STREET));
    /// assert!(!CellFlags::IS_STREET.contains(CellFlags::IS_PARK));
    /// ```
    #[must_use]
    pub const fn contains(self, rhs: CellFlags) -> bool {
        self.0 & rhs.0 == rhs.0
    }

    /// Return `self` with the `rhs` flags OR'd in.
    #[must_use]
    pub const fn insert(self, rhs: CellFlags) -> CellFlags {
        CellFlags(self.0 | rhs.0)
    }
}

/// A single city cell in `#[repr(C)]` layout, one per grid point.
///
/// Field order and padding match the C reference exactly so the struct can be
/// written straight to the wire and read by foreign consumers. See
/// `Urbix_Project.md` §2.3.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Cell {
    /// Building height in world units; `0` for street/open cells.
    pub height: f32,
    /// Zone-affinity weight per [`crate::zones::ZoneType`], length 5.
    pub zone_affinity: [f32; ZONE_COUNT],
    /// Facade color-palette index within the owning zone.
    pub palette_id: u8,
    /// Trait flags (see [`CellFlags`]).
    pub flags: CellFlags,
    /// Explicit 8-byte-alignment padding to match the C layout.
    pub _pad: u16,
    /// Deterministic interior key; `0` when the cell has no interior.
    pub interior_id: InteriorId,
}

/// Compile-time checks that the packed struct sizes match the C reference.
const _: () = {
    assert!(size_of::<Cell>() == 40, "Cell size drift from C layout");
    assert!(
        align_of::<Cell>() == 8,
        "Cell alignment drift from C layout"
    );
    assert!(
        size_of::<ChunkHeader>() == 32,
        "ChunkHeader size drift from C layout"
    );
    assert!(
        align_of::<ChunkHeader>() == 8,
        "ChunkHeader alignment drift"
    );
};

/// Header of a chunk buffer, matching the C reference layout.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct ChunkHeader {
    /// Chunk column index.
    pub cx: i32,
    /// Chunk row index.
    pub cy: i32,
    /// Total number of cells (`chunk_size × chunk_size`).
    pub cell_count: u32,
    /// Cells per chunk side (e.g. 32).
    pub chunk_size: u16,
    /// Padding to align `seed` on an 8-byte boundary.
    pub _pad: [u8; 6],
    /// World seed, kept for verification by the consumer.
    pub seed: u64,
}

/// Owned, packed byte stream of a chunk: header then cells, no padding.
///
/// This is the unit of data exchanged with consumers (via FFI or CLI). It owns
/// its bytes so that handing it across a boundary hands over ownership; the
/// matching `free`/drop releases it.
#[derive(Debug, Clone)]
pub struct ChunkBuffer {
    /// Packed bytes: `ChunkHeader` followed by `cell_count` `Cell` records.
    data: Vec<u8>,
}

impl ChunkBuffer {
    /// Build an owned chunk buffer: a header followed by `cell_count` cells.
    ///
    /// The cells are allocated as zeroed space up-front so the buffer has its
    /// full on-wire size; chunk generation (Milestone 3) fills them in.
    ///
    /// ## Example
    ///
    /// ```
    /// use urbix::data::{ChunkBuffer, ChunkId};
    ///
    /// let buf = ChunkBuffer::new(ChunkId::new(0, 0), 2, 42);
    /// assert_eq!(buf.header().cell_count, 4);
    /// assert_eq!(buf.header().seed, 42);
    /// ```
    #[must_use]
    pub fn new(id: ChunkId, chunk_size: u16, seed: u64) -> Self {
        let cell_count = u32::from(chunk_size) * u32::from(chunk_size);
        let header = ChunkHeader {
            cx: id.cx,
            cy: id.cy,
            cell_count,
            chunk_size,
            _pad: [0u8; 6],
            seed,
        };
        let cell_bytes = cell_count as usize * size_of::<Cell>();
        let mut data = Vec::with_capacity(size_of::<ChunkHeader>() + cell_bytes);
        // SAFETY: ChunkHeader is POD (#[repr(C)] with no padding holes we
        // rely on being zero); writing its bytes is well-defined.
        let header_slice = unsafe {
            std::slice::from_raw_parts(&header as *const _ as *const u8, size_of::<ChunkHeader>())
        };
        data.extend_from_slice(header_slice);
        // Extend the header out to its full on-wire size with zeroed cells so
        // the buffer length equals header + cell_count * sizeof(Cell).
        data.resize(size_of::<ChunkHeader>() + cell_bytes, 0);
        Self { data }
    }

    /// Decode the on-wire header from the buffer's leading bytes.
    #[must_use]
    pub fn header(&self) -> ChunkHeader {
        debug_assert!(self.data.len() >= size_of::<ChunkHeader>());
        // SAFETY: buffer always starts with a valid header written by `new`.
        unsafe { std::ptr::read_unaligned(self.data.as_ptr() as *const ChunkHeader) }
    }

    /// Borrow the packed bytes as a raw slice (header + cells).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::offset_of;

    #[test]
    fn cell_layout_matches_c_reference() {
        assert_eq!(size_of::<Cell>(), 40);
        assert_eq!(align_of::<Cell>(), 8);
        // Each field lands at the offset documented in Urbix_Project.md §2.3.
        assert_eq!(offset_of!(Cell, height), 0);
        assert_eq!(offset_of!(Cell, zone_affinity), 4);
        assert_eq!(offset_of!(Cell, palette_id), 24);
        assert_eq!(offset_of!(Cell, flags), 25);
        assert_eq!(offset_of!(Cell, interior_id), 32);
    }

    #[test]
    fn header_layout_matches_c_reference() {
        assert_eq!(size_of::<ChunkHeader>(), 32);
        assert_eq!(align_of::<ChunkHeader>(), 8);
        assert_eq!(offset_of!(ChunkHeader, cx), 0);
        assert_eq!(offset_of!(ChunkHeader, seed), 24);
    }

    #[test]
    fn chunk_buffer_roundtrips_header() {
        let buf = ChunkBuffer::new(ChunkId::new(-3, 7), 32, 445566);
        let h = buf.header();
        assert_eq!((h.cx, h.cy), (-3, 7));
        assert_eq!(h.chunk_size, 32);
        assert_eq!(h.cell_count, 1024);
        assert_eq!(h.seed, 445566);
        // Header + 1024 cells each 40 bytes.
        assert_eq!(buf.as_bytes().len(), 32 + 1024 * 40);
    }

    #[test]
    fn cell_flags_combine() {
        let f = CellFlags::default()
            .insert(CellFlags::IS_STREET)
            .insert(CellFlags::IS_PARK);
        assert!(f.contains(CellFlags::IS_STREET));
        assert!(f.contains(CellFlags::IS_PARK));
        assert!(!CellFlags::NONE.contains(CellFlags::IS_STREET));
    }
}
