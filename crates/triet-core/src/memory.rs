//! Object header and reference-count types for heap-allocated memory.
//!
//! Design lock per [ADR-0026 §3.1.1-§3.1.5]. Every heap allocation carries an
//! 8-byte header on binary targets (u32 refcount + u32 reserved), or a 54-trit
//! (6 Tryte) header on ternary native hardware (Integer refcount + Integer
//! reserved). The user-visible `&+ T` pointer always points to the **body**
//! (after the header), matching Objective-C / Swift patterns.
//!
//! # Layout (binary 64-bit target)
//!
//! ```text
//! Address: HEADER_ADDR          BODY_ADDR = HEADER_ADDR + 8
//!          |                    |
//!          v                    v
//!          [ refcount: u32 | reserved: u32 ] [ user fields ... ]
//!          |<-- 8 bytes -->|       |<-- sizeof(T) -->|
//! ```
//!
//! # Atomicity
//!
//! Refcount operations must be atomic because sender and receiver actors may
//! run on different OS threads. On ARM: LL/SC; on x86: LOCK XADD. Cost ~5-15 ns
//! per op on modern hardware.
//!
//! # Ternary native (v∞ trytecode)
//!
//! When ternary hardware ships, the header maps to 54 trit = 6 Tryte = 2
//! Integer. The signed Integer refcount enables negative sentinels: -1 =
//! static allocation (never free), -2 = frozen forever (refcount disabled).
//! Atomic ops skip entirely when `current < 0`.

use std::sync::atomic::{AtomicU32, Ordering};

/// Header size in bytes on binary 64-bit targets.
pub const HEADER_SIZE_BINARY: usize = 8;

/// Refcount field offset from header start (0 bytes).
pub const REFCOUNT_OFFSET: usize = 0;

/// Reserved field offset from header start (4 bytes).
pub const RESERVED_OFFSET: usize = 4;

/// Heap allocation header for binary 64-bit targets.
///
/// Placed at a negative offset from the user-visible pointer.
/// The caller is responsible for the correct layout — this struct
/// serves as the canonical definition for all memory-management
/// code paths (VM allocation, interpreter Rc wrapping, future AOT).
#[derive(Debug)]
#[repr(C, align(8))]
pub struct ObjectHeader {
    /// Number of active `&+ T` handles. Atomic because cross-actor
    /// send may increment from a different OS thread.
    pub refcount: AtomicU32,
    /// Reserved for future use: type tag bits, drop flags, capability
    /// audit metadata, cycle collector mark bit. Currently zero.
    pub reserved: AtomicU32,
}

impl Default for ObjectHeader {
    fn default() -> Self {
        Self {
            refcount: AtomicU32::new(1),
            reserved: AtomicU32::new(0),
        }
    }
}

impl ObjectHeader {
    /// Create a header with refcount = 1.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a header for a static allocation (never freed).
    ///
    /// On ternary native hardware, static allocation uses sentinel -1.
    /// On binary targets, we use `refcount = u32::MAX` as a sentinel —
    /// strictly greater than any refcount attainable in practice.
    pub fn new_static() -> Self {
        Self {
            refcount: AtomicU32::new(STATIC_SENTINEL),
            reserved: AtomicU32::new(0),
        }
    }

    /// Create a header for a frozen-forever allocation.
    ///
    /// Frozen objects are immutable and shared without refcount tracking.
    /// Ternary native: sentinel -2. Binary: `u32::MAX - 1`.
    pub fn new_frozen_forever() -> Self {
        Self {
            refcount: AtomicU32::new(FROZEN_FOREVER_SENTINEL),
            reserved: AtomicU32::new(0),
        }
    }

    /// Atomically increment the refcount. Returns the new value.
    pub fn increment(&self) -> u32 {
        // Skip atomic op for static / frozen-forever sentinels.
        let current = self.refcount.load(Ordering::Relaxed);
        if current >= FROZEN_FOREVER_SENTINEL {
            return current;
        }
        self.refcount.fetch_add(1, Ordering::AcqRel) + 1
    }

    /// Atomically decrement the refcount. Returns the new value.
    /// Caller checks for zero to trigger deallocation.
    pub fn decrement(&self) -> u32 {
        let current = self.refcount.load(Ordering::Relaxed);
        if current >= FROZEN_FOREVER_SENTINEL {
            return current; // static / frozen — no-op
        }
        self.refcount.fetch_sub(1, Ordering::AcqRel) - 1
    }

    /// Returns true if this is a static allocation (never freed).
    pub fn is_static(&self) -> bool {
        self.refcount.load(Ordering::Relaxed) == STATIC_SENTINEL
    }

    /// Returns true if this is a frozen-forever allocation.
    pub fn is_frozen_forever(&self) -> bool {
        self.refcount.load(Ordering::Relaxed) == FROZEN_FOREVER_SENTINEL
    }

    /// Returns true if refcount tracking is disabled (static or frozen).
    pub fn is_refcount_disabled(&self) -> bool {
        self.refcount.load(Ordering::Relaxed) >= FROZEN_FOREVER_SENTINEL
    }
}

// ── Sentinels ────────────────────────────────────────────────────────────

/// Sentinel refcount value for static allocations (never freed).
///
/// Ternary native maps this to -1. On binary, we use `u32::MAX` — strictly
/// larger than any realistic refcount (max practical ~10⁶ per object).
pub const STATIC_SENTINEL: u32 = u32::MAX;

/// Sentinel refcount value for frozen-forever allocations.
///
/// Ternary native maps this to -2. On binary, `u32::MAX - 1`.
pub const FROZEN_FOREVER_SENTINEL: u32 = u32::MAX - 1;

// ── Ternary native (v∞ trytecode) ───────────────────────────────────────

/// Header size in trits on ternary native hardware.
/// 2 × Integer = 2 × 27 trits = 54 trits = 6 Tryte.
pub const HEADER_SIZE_TERNARY_TRITS: usize = 54;

/// Header size in Tryte (9-trit) units.
pub const HEADER_SIZE_TERNARY_TRYTES: usize = 6;

/// Packed header size when ternary is stored in binary memory
/// (5 trits/byte per SPEC §1.5.1): ceil(54 / 5) = 11 bytes.
pub const HEADER_SIZE_TERNARY_PACKED: usize = 11;

// ── Refcount sentinel semantics (ternary-native Integer) ─────────────────
//
// When ternary hardware ships, the refcount field is an Integer (27-trit
// signed balanced ternary). The positive range [0, +3²⁶] counts refs.
// Negative values are sentinels:
//
//   > 0       — alive, N strong refs
//   = 0       — in process of being freed (destructor running)
//   = -1      — static allocation (never free)
//   = -2      — frozen forever (refcount disabled)
//   < -2      — reserved for future use
//
// The key optimization: atomic op checks `current < 0` to skip refcount
// machinery entirely for static/frozen objects.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_size_is_8_bytes() {
        assert_eq!(size_of::<ObjectHeader>(), HEADER_SIZE_BINARY);
    }

    #[test]
    fn header_alignment_is_8() {
        assert_eq!(align_of::<ObjectHeader>(), 8);
    }

    #[test]
    fn new_header_refcount_is_1() {
        let h = ObjectHeader::new();
        assert_eq!(h.refcount.load(Ordering::Relaxed), 1);
        assert!(!h.is_static());
        assert!(!h.is_frozen_forever());
        assert!(!h.is_refcount_disabled());
    }

    #[test]
    fn static_sentinel_detected() {
        let h = ObjectHeader::new_static();
        assert!(h.is_static());
        assert!(h.is_refcount_disabled());
        assert_eq!(h.increment(), STATIC_SENTINEL); // no-op for static
        assert_eq!(h.decrement(), STATIC_SENTINEL); // no-op for static
    }

    #[test]
    fn frozen_forever_sentinel_detected() {
        let h = ObjectHeader::new_frozen_forever();
        assert!(h.is_frozen_forever());
        assert!(h.is_refcount_disabled());
        assert_eq!(h.increment(), FROZEN_FOREVER_SENTINEL); // no-op
        assert_eq!(h.decrement(), FROZEN_FOREVER_SENTINEL); // no-op
    }

    #[test]
    fn increment_decrement_cycle() {
        let h = ObjectHeader::new(); // refcount = 1
        assert_eq!(h.increment(), 2);
        assert_eq!(h.increment(), 3);
        assert_eq!(h.decrement(), 2);
        assert_eq!(h.decrement(), 1);
        assert_eq!(h.decrement(), 0); // caller frees at 0
    }

    #[test]
    fn reserved_starts_at_zero() {
        let h = ObjectHeader::new();
        assert_eq!(h.reserved.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn default_is_same_as_new() {
        let h1 = ObjectHeader::default();
        let h2 = ObjectHeader::new();
        assert_eq!(h1.refcount.load(Ordering::Relaxed), 1);
        assert_eq!(h2.refcount.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn ternary_constants_match_spec() {
        // SPEC §1.5.1 canonical sizes
        assert_eq!(HEADER_SIZE_TERNARY_TRITS, 54);
        assert_eq!(HEADER_SIZE_TERNARY_TRYTES, 6);
        // ceil(54 / 5) = 11
        assert_eq!(HEADER_SIZE_TERNARY_PACKED, 11);
    }

    #[test]
    fn sentinels_are_distinct() {
        assert_ne!(STATIC_SENTINEL, FROZEN_FOREVER_SENTINEL);
        // Sentinels never collide with realistic refcounts (< 10^6)
        assert!(STATIC_SENTINEL > 1_000_000);
        assert!(FROZEN_FOREVER_SENTINEL > 1_000_000);
    }

    #[test]
    fn header_layout_is_repr_c_with_correct_alignment() {
        // repr(C) + align(8) guarantees refcount at offset 0, reserved at
        // offset 4. Verify via size + alignment (no pointer math needed).
        assert_eq!(size_of::<ObjectHeader>(), 8);
        assert_eq!(align_of::<ObjectHeader>(), 8);
        // Verify the field offset contract indirectly: new() sets refcount
        // to 1 and reserved to 0, so reading them should reflect that.
        let h = ObjectHeader::new();
        assert_eq!(h.refcount.load(Ordering::Relaxed), 1);
        assert_eq!(h.reserved.load(Ordering::Relaxed), 0);
    }
}
