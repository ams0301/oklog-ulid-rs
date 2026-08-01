// SPDX-License-Identifier: Apache-2.0
//
// Core ULID type, byte layout, and time accessors.
// Ports reference/ulid.go lines 48-78, 407-458, 391-393.

use crate::{Error, Result};

/// Length of the raw binary ULID, in bytes.
///
/// Mirrors Go source `len(ULID)` and the array backing `[16]byte`.
pub const RAW_SIZE: usize = 16;

/// Length of the text-encoded ULID, in characters.
///
/// Mirrors Go sentinel `EncodedSize = 26`.
pub const ENCODED_SIZE: usize = 26;

/// Marker for the `MaxTime` constructor — see [`Ulid::time`] and
/// [`MaxTime::VALUE`].
pub struct MaxTime;

impl MaxTime {
    /// Maximum Unix time in milliseconds representable in a ULID, equal to
    /// `(1 << 48) - 1` = `281474976710655` = `0xFFFFFFFFFFFF`.
    ///
    /// Mirrors Go `var maxTime = ULID{0xFF,..}.Time()` and `MaxTime()` func.
    /// Computed at compile time from the byte layout rather than read off a
    /// sentinel instance, but the numeric value is identical.
    pub const VALUE: u64 = (1u64 << 48) - 1;

    /// Returns [`Self::VALUE`]. Convenience form matching the Go API
    /// `MaxTime() uint64`.
    #[inline]
    pub const fn get() -> u64 {
        Self::VALUE
    }
}

/// A 16-byte Universally Unique Lexicographically Sortable Identifier.
///
/// Bytes 0..=5 are a 48-bit Unix-millisecond timestamp, big-endian.
/// Bytes 6..=15 are 80 bits of entropy (random or monotonic).
///
/// `Ulid` is `Copy`, `Eq`, `Ord`, and zero-cost to pass by value. It is
/// safe to construct via [`Ulid::ZERO`] or by parsing a string. The
/// `io.Reader`-driven constructors (`New`/`MustNew`/`Make`) are added in
/// a later commit alongside the entropy trait.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default, Debug)]
#[repr(transparent)]
pub struct Ulid(pub(crate) [u8; RAW_SIZE]);

impl Ulid {
    /// The all-zero ULID (`00000000000000000000000000`).
    pub const ZERO: Ulid = Ulid([0; RAW_SIZE]);

    /// Construct a ULID from raw big-endian bytes.
    #[inline]
    pub const fn from_bytes(b: [u8; RAW_SIZE]) -> Self {
        Ulid(b)
    }

    /// View the ULID as its raw 16 big-endian bytes.
    ///
    /// Matches Go `id.Bytes()` — except we return a reference rather than
    /// a fresh allocation, since the Go `Bytes()` returns `id[:]` (a slice
    /// into the underlying array). Use [`Ulid::to_bytes`] for a copy.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8; RAW_SIZE] {
        &self.0
    }

    /// Copy the ULID out as an owned 16-byte array.
    ///
    /// For test compatibility with `TestULID_Bytes` in the Go suite which
    /// proves `Bytes()` returns an independent slice, this is the
    /// copy-returning form the port exposes by default; the `.as_bytes()`
    /// reference form lives behind it.
    #[inline]
    pub const fn to_bytes(&self) -> [u8; RAW_SIZE] {
        self.0
    }

    /// Return the Unix time in milliseconds encoded in the ULID.
    ///
    /// Mirrors Go `func (id ULID) Time() uint64`. Big-endian across bytes
    /// 0..=5, see comment in [`Ulid`] for layout.
    #[inline]
    pub const fn time(&self) -> u64 {
        (self.0[0] as u64) << 40
            | (self.0[1] as u64) << 32
            | (self.0[2] as u64) << 24
            | (self.0[3] as u64) << 16
            | (self.0[4] as u64) << 8
            | (self.0[5] as u64)
    }

    /// Set the 48-bit time component to `ms` (Unix milliseconds).
    ///
    /// Returns [`Error::BigTime`] when `ms > MaxTime::VALUE`, mirroring the
    /// Go `ErrBigTime` sentinel.
    ///
    /// Mirrors Go `func (id *ULID) SetTime(ms uint64) error`.
    #[inline]
    pub fn set_time(&mut self, ms: u64) -> Result<()> {
        if ms > MaxTime::VALUE {
            return Err(Error::BigTime);
        }
        self.0[0] = (ms >> 40) as u8;
        self.0[1] = (ms >> 32) as u8;
        self.0[2] = (ms >> 24) as u8;
        self.0[3] = (ms >> 16) as u8;
        self.0[4] = (ms >> 8) as u8;
        self.0[5] = ms as u8;
        Ok(())
    }

    /// `const`-context variant of [`Ulid::set_time`] that panics on overflow.
    ///
    /// Used to express the `MaxTime`-derived constants at compile time
    /// without needing `弧度` runtime evaluation. Mirrors the Go zero-cost
    /// `var maxTime = ULID{0xFF,..}.Time()` initialization.
    #[inline]
    pub const fn with_time_checked(mut self, ms: u64) -> Self {
        debug_assert!(ms <= MaxTime::VALUE, "ulid: time too big (const ctor)");
        self.0[0] = (ms >> 40) as u8;
        self.0[1] = (ms >> 32) as u8;
        self.0[2] = (ms >> 24) as u8;
        self.0[3] = (ms >> 16) as u8;
        self.0[4] = (ms >> 8) as u8;
        self.0[5] = ms as u8;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors Go test `TestULIDTime` lines 388-411: set random ms values
    /// within `[0, maxTime]`, read back via `Time()`, ensure round-trips.
    /// The Go suite uses 1e6 random draws; we use a small LCG to avoid
    /// a `rand` dependency in the core crate. Equivalent semantics.
    #[test]
    fn set_time_round_trips_across_ms_range() {
        let max_time = MaxTime::VALUE;
        let mut id = Ulid::ZERO;
        // First check the boundary: max_time + 1 must reject.
        assert_eq!(id.set_time(max_time + 1), Err(Error::BigTime));

        // Crude xorshift-style LCG for deterministic coverage of [0, max).
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..1_000_000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let ms = seed % max_time;
            let mut id = Ulid::ZERO;
            id.set_time(ms).expect("ms <= max_time");
            assert_eq!(id.time(), ms, "round-trip failed for ms={ms}");
        }
    }

    /// Boundary: `MaxTime` itself must round-trip.
    #[test]
    fn max_time_round_trips() {
        let mut id = Ulid::ZERO;
        id.set_time(MaxTime::VALUE).unwrap();
        assert_eq!(id.time(), MaxTime::VALUE);
        // All six time bytes are 0xFF at the upper boundary.
        assert_eq!(&id.as_bytes()[..6], &[0xFF; 6]);
    }

    /// Bytes 0..=5 are time MSB-first; bytes 6..=15 are entropy and must
    /// remain untouched by `set_time`.
    #[test]
    fn set_time_does_not_touch_entropy() {
        let mut id = Ulid::from_bytes([
            0, 0, 0, 0, 0, 0, //
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33, 0x44,
        ]);
        id.set_time(0x0123_4567_89AB).unwrap();
        // Time bytes:
        assert_eq!(id.as_bytes()[..6], [0x01, 0x23, 0x45, 0x67, 0x89, 0xAB]);
        // Entropy untouched:
        assert_eq!(&id.as_bytes()[6..], &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33, 0x44]);
    }
}
