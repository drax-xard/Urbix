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

use std::mem::{align_of, offset_of, size_of};

use serde::{Deserialize, Serialize};

/// Number of zone-affinity weights stored per cell. Sourced from
/// [`crate::zones::ZONE_COUNT`] so the wire format never drifts from the zone
/// definition.
pub use crate::zones::ZONE_COUNT;

/// Identifier addressing a chunk by its integer `(cx, cy)` grid coordinates.
///
/// `#[repr(C)]` and hashable so it can serve as both an FFI-visible record and
/// an LRU-cache key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(skip, default)]
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
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
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
    #[serde(skip, default)]
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
        let cell_bytes = cell_count as usize * size_of::<Cell>();
        let total = size_of::<ChunkHeader>() + cell_bytes;

        // Zero-init the WHOLE buffer up-front. The `ChunkHeader` struct has
        // implicit alignment padding between `_pad` and `seed` that a struct
        // literal cannot name; copying raw struct bytes would smuggle
        // uninitialized stack padding into the wire format (UB + breaking the
        // determinism invariant). Starting from zeros keeps every padding byte
        // deterministic and zero.
        let mut data = vec![0u8; total];

        // Write the header fields at their exact on-wire offsets, unaligned,
        // leaving the (already-zeroed) implicit alignment padding untouched.
        // Offsets come from `offset_of!` so they can never drift from the real
        // layout. `seed` sits after 6 explicit `_pad` bytes plus implicit
        // padding, at offset 24.
        unsafe {
            let p = data.as_mut_ptr();
            std::ptr::write_unaligned(p.add(offset_of!(ChunkHeader, cx)).cast::<i32>(), id.cx);
            std::ptr::write_unaligned(p.add(offset_of!(ChunkHeader, cy)).cast::<i32>(), id.cy);
            std::ptr::write_unaligned(
                p.add(offset_of!(ChunkHeader, cell_count)).cast::<u32>(),
                cell_count,
            );
            std::ptr::write_unaligned(
                p.add(offset_of!(ChunkHeader, chunk_size)).cast::<u16>(),
                chunk_size,
            );
            std::ptr::write_unaligned(p.add(offset_of!(ChunkHeader, seed)).cast::<u64>(), seed);
        }

        Self { data }
    }

    /// Decode the on-wire header from the buffer's leading bytes.
    ///
    /// ## Panics
    ///
    /// Panics if the buffer holds fewer bytes than a `ChunkHeader` (a defensive
    /// guard against malformed buffers, e.g. over FFI).
    #[must_use]
    pub fn header(&self) -> ChunkHeader {
        assert!(
            self.data.len() >= size_of::<ChunkHeader>(),
            "buffer shorter than a ChunkHeader (len {})",
            self.data.len()
        );
        // SAFETY: length checked above; the read is unaligned so it stays valid
        // regardless of allocation alignment. The header was written by `new`.
        unsafe { std::ptr::read_unaligned(self.data.as_ptr() as *const ChunkHeader) }
    }

    /// Borrow the packed bytes as a raw slice (header + cells).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Leak the buffer's bytes into a raw `(data, len)` pair for hand-off
    /// across an FFI boundary.
    ///
    /// The returned buffer lives in the Rust allocator and must be reclaimed
    /// with [`from_raw_bytes`](Self::from_raw_bytes) (or the FFI
    /// `urbix_chunk_free`) — never freed by a foreign allocator. Length is the
    /// full on-wire size (header + cells).
    #[must_use]
    pub fn into_raw_bytes(self) -> (*mut u8, usize) {
        // `Vec -> Box<[u8]>` guarantees a slice whose total size is exactly
        // len bytes with no spare capacity, so reclaiming it later only needs
        // the (ptr, len) we return — no capacity bookkeeping to smuggle across
        // the boundary.
        let boxed: Box<[u8]> = self.data.into_boxed_slice();
        let fat = Box::into_raw(boxed); // *mut [u8]
        let len = fat.len();
        (fat.cast::<u8>(), len)
    }

    /// Claim back a buffer that was leaked by [`into_raw_bytes`](Self::into_raw_bytes).
    ///
    /// # Safety
    ///
    /// `ptr`/`len` must come from a single unmatched `into_raw_bytes` call;
    /// calling this twice on the same pointer (or with an unrelated pointer) is
    /// double-free / undefined behaviour.
    #[must_use]
    pub unsafe fn from_raw_bytes(ptr: *mut u8, len: usize) -> Self {
        // Rebuild the exact boxed slice and hand it back to the Vec it came from.
        let fat = std::ptr::slice_from_raw_parts_mut(ptr, len);
        // SAFETY: caller guarantees `ptr`/`len` came from `into_raw_bytes`.
        let boxed: Box<[u8]> = Box::from_raw(fat);
        Self {
            data: boxed.into_vec(),
        }
    }

    /// Number of `Cell` records carried by this buffer.
    #[must_use]
    pub fn cell_count(&self) -> usize {
        self.header().cell_count as usize
    }

    /// Index of the first cell byte relative to the start of the buffer.
    fn cell_region_offset(&self) -> usize {
        size_of::<ChunkHeader>()
    }

    /// Read the `index`-th cell's packed bytes into a `Cell` value.
    ///
    /// ## Panics
    ///
    /// Panics if `index >= cell_count()`, or if the buffer's actual byte
    /// length is inconsistent with the header (a defensive guard against
    /// malformed buffers handed in from outside, e.g. over FFI).
    #[must_use]
    pub fn get_cell(&self, index: usize) -> Cell {
        assert!(index < self.cell_count(), "cell index out of range");
        let off = self.cell_region_offset() + index * size_of::<Cell>();
        let end = off + size_of::<Cell>();
        assert!(
            end <= self.data.len(),
            "buffer is shorter than its header claims (off {off}, len {})",
            self.data.len()
        );
        // SAFETY: bounds checked above; the read is unaligned so it stays valid
        // regardless of allocation alignment.
        unsafe { std::ptr::read_unaligned(self.data.as_ptr().add(off) as *const Cell) }
    }

    /// Write a `Cell` into the `index`-th slot of the packed buffer.
    ///
    /// ## Panics
    ///
    /// Panics if `index >= cell_count()`, or if the buffer's actual byte
    /// length is inconsistent with the header (a defensive guard against
    /// malformed buffers handed in from outside, e.g. over FFI).
    pub fn set_cell(&mut self, index: usize, cell: Cell) {
        assert!(index < self.cell_count(), "cell index out of range");
        let off = self.cell_region_offset() + index * size_of::<Cell>();
        let end = off + size_of::<Cell>();
        assert!(
            end <= self.data.len(),
            "buffer is shorter than its header claims (off {off}, len {})",
            self.data.len()
        );
        // SAFETY: bounds checked above; written unaligned so it is valid for
        // any allocation alignment. `Cell` is POD and contains no references,
        // so copying its bytes is safe.
        unsafe {
            std::ptr::write_unaligned(self.data.as_mut_ptr().add(off) as *mut Cell, cell);
        }
    }

    /// Iterate over all cells as freshly-read `Cell` values.
    ///
    /// Useful for consumer code and tests that want to scan a whole chunk
    /// without mutating it.
    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (0..self.cell_count()).map(move |i| self.get_cell(i))
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

    #[test]
    fn chunk_buffer_cell_roundtrip() {
        let mut buf = ChunkBuffer::new(ChunkId::new(0, 0), 2, 42);
        let cell = Cell {
            height: 12.5,
            zone_affinity: [0.2, 0.3, 0.5, 0.0, 0.0],
            palette_id: 3,
            flags: CellFlags::IS_STREET,
            _pad: 0,
            interior_id: 99,
        };
        buf.set_cell(0, cell);
        assert_eq!(buf.get_cell(0), cell);
        // Unwritten cells stay zeroed.
        assert_eq!(buf.get_cell(1).height, 0.0);
        assert_eq!(buf.cell_count(), 4);
        assert_eq!(buf.cells().count(), 4);
    }

    #[test]
    #[should_panic(expected = "shorter")]
    fn get_cell_rejects_truncated_buffer() {
        let mut buf = ChunkBuffer::new(ChunkId::new(0, 0), 4, 42);
        // Shrink the backing vec so the header's cell_count (16) no longer
        // matches the actual bytes held — the accessor must refuse to read OOB.
        buf.data.truncate(size_of::<ChunkHeader>() + 40);
        let _ = buf.get_cell(15);
    }

    #[test]
    #[should_panic(expected = "shorter")]
    fn set_cell_rejects_truncated_buffer() {
        let mut buf = ChunkBuffer::new(ChunkId::new(0, 0), 4, 42);
        buf.data.truncate(size_of::<ChunkHeader>() + 40);
        let cell = Cell {
            height: 1.0,
            zone_affinity: [0.0; ZONE_COUNT],
            palette_id: 0,
            flags: CellFlags::NONE,
            _pad: 0,
            interior_id: 0,
        };
        buf.set_cell(15, cell);
    }

    #[test]
    fn raw_bytes_round_trip_preserves_content() {
        let buf = ChunkBuffer::new(ChunkId::new(3, -7), 8, 99);
        let expected = buf.as_bytes().to_vec();
        let (ptr, len) = buf.into_raw_bytes();
        assert_eq!(len, expected.len());
        // Reclaim and confirm byte-for-byte identity.
        // SAFETY: ptr/len come from the single unmatched into_raw_bytes call.
        let reclaimed = unsafe { ChunkBuffer::from_raw_bytes(ptr, len) };
        assert_eq!(reclaimed.as_bytes(), expected.as_slice());
        assert_eq!(reclaimed.header().cell_count, 8 * 8);
        // The reclaimed buffer is fully usable again.
        let cell = reclaimed.get_cell(0);
        assert_eq!(cell.zone_affinity, [0.0f32; ZONE_COUNT]);
        assert_eq!(cell.height, 0.0);
    }
}
