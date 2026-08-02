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

    /// View the ULID as a mutable reference to its raw 16 big-endian bytes.
    ///
    /// Matches the Go pattern `id[6:]` where callers (notably the CLI in
    /// `cmd/ulid/main.go` and `DefaultEntropy/Make`) write entropy bytes
    /// directly into the tail of the array. The companion to
    /// [`Ulid::as_bytes`] for the write path.
    #[inline]
    pub fn as_bytes_mut(&mut self) -> &mut [u8; RAW_SIZE] {
        &mut self.0
    }

    /// Lexicographic comparison — matches Go `func (id ULID) Compare(other ULID) int`
    /// which delegates to `bytes.Compare(id[:], other[:])`. Because ULID
    /// bytes are big-endian over the whole 128-bit payload, byte-wise
    /// comparison equals numeric comparison, identical to the Go behaviour.
    ///
    /// The Rust idiom is `Ulid: Ord` (derived), giving `id.cmp(&other)`;
    /// this method is the named Go-API counterpart returning an
    /// [`Ordering`] that integrates with both idioms: `id.compare(other)
    /// == Ordering::Less` works just as `id.cmp(&other)` does.
    #[inline]
    pub fn compare(&self, other: &Ulid) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
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

    /// Return the 10-byte entropy tail. Mirrors Go `func (id ULID) Entropy() []byte`.
    ///
    /// The Go original returns a slice into the underlying array; we return
    /// an owned `[u8; 10]` to mirror the `bytes()` decision (by-value keeps
    /// the type `Copy` and the slice-mutation semantics that `TestULID_Bytes`
    /// pins down are still preserved because `set_entropy` is the only
    /// write path). For a borrowed view use `id.as_bytes()[6..].try_into().unwrap()`.
    #[inline]
    pub const fn entropy(&self) -> [u8; 10] {
        let mut out = [0u8; 10];
        out[0] = self.0[6];
        out[1] = self.0[7];
        out[2] = self.0[8];
        out[3] = self.0[9];
        out[4] = self.0[10];
        out[5] = self.0[11];
        out[6] = self.0[12];
        out[7] = self.0[13];
        out[8] = self.0[14];
        out[9] = self.0[15];
        out
    }

    /// Set the 10-byte entropy tail. Mirrors Go `func (id *ULID) SetEntropy(e []byte) error`.
    ///
    /// Returns [`Error::DataSize`] when `entropy.len() != 10`, the same
    /// sentinel as the Go original. Reading back via [`Ulid::entropy`]
    /// round-trips for any 10-byte input.
    #[inline]
    pub fn set_entropy(&mut self, entropy: &[u8]) -> Result<()> {
        if entropy.len() != 10 {
            return Err(Error::DataSize);
        }
        self.0[6..16].copy_from_slice(entropy);
        Ok(())
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
        assert_eq!(
            &id.as_bytes()[6..],
            &[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33, 0x44]
        );
    }
}
