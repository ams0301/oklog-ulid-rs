// SPDX-License-Identifier: Apache-2.0
//
// Monotonic entropy: 80-bit accumulator + a source wrapper that yields
// strictly-increasing entropy within the same ms timestamp.
//
// Ports reference/ulid.go lines 535-696 (Monotonic + MonotonicEntropy
// + LockedMonotonicReader + uint80 + increment + random).
//
// Behaviour:
//   * The first call for a given `ms` reads 10 bytes from the
//     underlying source and stores them as a `uint80`.
//   * Subsequent calls with the same `ms` add a random `inc` in
//     [1, m.inc] to the previous entropy value and write back.
//   * If incrementing overflows 2^80 - 1, `ErrMonotonicOverflow`
//     is returned (the Go suite covers this in TestMonotonicOverflow).
//
// `Locked<T>` wraps any `Monotonic` with a `Mutex` for thread-safe
// use, mirroring Go `LockedMonotonicReader` (lines 570-583). The
// `DefaultEntropy` factory in `sys.rs` (Port #6) builds one of these
// around a `MonotonicEntropy<bufio::BufReader<rand>>`.

use crate::entropy::{Entropy, Monotonic};
use crate::ulid::RAW_SIZE;
use crate::{Error, Result};

/// 80-bit unsigned accumulator laid out as a 16-bit high word and a
/// 64-bit low word. Mirrors Go `type uint80 struct { Hi uint16; Lo uint64 }`
/// (reference/ulid.go lines 671-674).
#[derive(Copy, Clone, Default, Debug, PartialEq, Eq)]
pub struct Uint80 {
    pub hi: u16,
    pub lo: u64,
}

impl Uint80 {
    /// `Uint80::ZERO` convenience.
    pub const ZERO: Self = Uint80 { hi: 0, lo: 0 };

    /// True iff both halves are zero. Mirrors Go `func (u uint80) IsZero() bool`
    /// (lines 694-696).
    #[inline]
    pub const fn is_zero(&self) -> bool {
        self.hi == 0 && self.lo == 0
    }

    /// Read the big-endian byte representation of an 80-bit value into
    /// the accumulator. Mirrors Go `func (u *uint80) SetBytes(bs []byte)`
    /// (lines 676-679). Panics if `bs.len() < 10`.
    #[inline]
    pub fn set_bytes(&mut self, bs: &[u8]) {
        debug_assert!(bs.len() >= 10, "uint80::set_bytes needs 10 bytes");
        // Two most-significant bytes form `Hi`; the next 8 form `Lo`.
        self.hi = (bs[0] as u16) << 8 | bs[1] as u16;
        self.lo = u64::from_be_bytes([bs[2], bs[3], bs[4], bs[5], bs[6], bs[7], bs[8], bs[9]]);
    }

    /// Write the accumulator into 10 big-endian bytes. Mirrors Go
    /// `func (u *uint80) AppendTo(bs []byte)` (lines 681-684).
    #[inline]
    pub fn append_to(&self, bs: &mut [u8]) {
        debug_assert!(bs.len() >= 10, "uint80::append_to needs 10 bytes");
        bs[0] = (self.hi >> 8) as u8;
        bs[1] = self.hi as u8;
        bs[2..10].copy_from_slice(&self.lo.to_be_bytes());
    }

    /// Add `n` to the accumulator. Returns `true` if the high word
    /// *decreased* — i.e. the addition overflowed past 2^80.
    ///
    /// Mirrors Go `func (u *uint80) Add(n uint64) (overflow bool)`
    /// (lines 686-692). The Go check is `u.Hi < hi` (the saved old Hi)
    /// because both halves get mutated in lockstep before the comparison
    /// returns. Translating literally:
    ///
    ///   1. Save `lo = u.Lo`, `hi = u.Hi`.
    ///   2. `u.Lo += n`; if the new `u.Lo` < the saved `lo`, increment `u.Hi`.
    ///   3. Return `u.Hi < hi`.
    ///
    /// Identical semantics whether `hi` wraps after the Lo-add (which is
    /// the only path that can change Hi) and identical overflow-report
    /// criterion.
    #[inline]
    pub fn add(&mut self, n: u64) -> bool {
        let old_lo = self.lo;
        let old_hi = self.hi;
        self.lo = self.lo.wrapping_add(n);
        if self.lo < old_lo {
            // Carry into Hi. wrapping_add on a u16 matches Go `u.Hi++`.
            self.hi = self.hi.wrapping_add(1);
        }
        // Overflow iff Hi decreased — i.e. wrapped.
        self.hi < old_hi
    }
}

/// Monotonic entropy source. Mirrors Go `type MonotonicEntropy struct`
/// (lines 585-593). The inner reader is buffered in the Go version via
/// `bufio.NewReader`; the Rust port uses a small `[u8; 8]` workspace
/// directly since `Entropy::read` already abstracts the byte stream.
#[derive(Debug)]
pub struct MonotonicEntropy<E> {
    pub inner: E,
    /// Most recent ms we generated entropy for. `0` means fresh/initial.
    ms: u64,
    /// Last entropy value as an 80-bit accumulator.
    entropy: Uint80,
    /// Increment bound — the random component added per same-ms call is
    /// drawn from `[1, inc]`. Default `0` is replaced with
    /// `math.MaxUint32` by the constructor, matching Go `Monotonic` at
    /// lines 551-566.
    pub inc: u64,
    /// Workspace scratch buffer for `random()`'s byte draws. Mirrors the
    /// Go 8-byte `rand` field (line 592).
    rand_buf: [u8; 8],
}

impl<E: Entropy> MonotonicEntropy<E> {
    /// Build a monotonic entropy source on top of `inner`, with the
    /// given increment bound. `inc == 0` is converted to `u32::MAX` per
    /// the Go comment at lines 543-549 (default `math.MaxUint32`).
    ///
    /// Mirrors Go `func Monotonic(entropy io.Reader, inc uint64) *MonotonicEntropy`.
    pub fn new(inner: E, inc: u64) -> Self {
        MonotonicEntropy {
            inner,
            ms: 0,
            entropy: Uint80::ZERO,
            inc: if inc == 0 { u32::MAX as u64 } else { inc },
            rand_buf: [0u8; 8],
        }
    }

    /// Implements the same body as Go `func (m *MonotonicEntropy)
    /// MonotonicRead(ms uint64, entropy []byte) (err error)` (lines 596-605):
    ///
    ///   if !m.entropy.IsZero() && m.ms == ms:
    ///       err = m.increment()
    ///       m.entropy.AppendTo(entropy)
    ///   else if ReadFull(m.Reader, entropy):
    ///       m.ms = ms
    ///       m.entropy.SetBytes(entropy)
    fn monotonic_read_inner(&mut self, ms: u64, dst: &mut [u8]) -> Result<()> {
        if dst.len() < RAW_SIZE - 6 {
            return Err(Error::BufferSize);
        }
        // Take only the first 10 bytes of the 10-byte entropy slot
        // (ULID entropy is exactly 10 bytes).
        let entropy_len = RAW_SIZE - 6;
        let dst10 = &mut dst[..entropy_len];

        if !self.entropy.is_zero() && self.ms == ms {
            // Increment-then-write path. Matches Go `m.increment()` +
            // `m.entropy.AppendTo(entropy)`.
            self.increment()?;
            self.entropy.append_to(dst10);
        } else {
            // Fresh entropy: read 10 bytes from the underlying source.
            self.inner.read(dst10)?;
            self.ms = ms;
            self.entropy.set_bytes(dst10);
        }
        Ok(())
    }

    /// Increment the previous entropy by a random in `[1, m.inc]`. On
    /// overflow of `m.entropy` past 2^80 - 1, return `ErrMonotonicOverflow`.
    /// Mirrors Go `func (m *MonotonicEntropy) increment() error` (lines 609-616).
    fn increment(&mut self) -> Result<()> {
        let inc = self.random()?;
        if self.entropy.add(inc) {
            return Err(Error::MonotonicOverflow);
        }
        Ok(())
    }

    /// Pick a uniform `inc` in `[1, self.inc)`, drawing from the
    /// underlying source. If `self.inc <= 1`, returns 1 without reading
    /// (matching Go's early-out at lines 621-624).
    ///
    /// Pillar of the Go `random()` function (lines 620-669); the byte
    /// routing is a direct port, using `u64::from_le_bytes` in place of
    /// `binary.LittleEndian.Uint*`.
    fn random(&mut self) -> Result<u64> {
        if self.inc <= 1 {
            return Ok(1);
        }
        let bit_len = (64 - (self.inc - 1).leading_zeros()) as usize; // bits.Len64(self.inc)
        let byte_len = bit_len.div_ceil(8);
        let msbit_len = if bit_len.is_multiple_of(8) {
            8
        } else {
            bit_len % 8
        };

        let mut inc: u64 = 0;
        while inc == 0 || inc >= self.inc {
            // Pull `byte_len` bytes from the inner source. Truncate the
            // first byte's high bits so candidate values concentrate in
            // `[0, self.inc)` rather than across all u64s.
            self.inner.read(&mut self.rand_buf[..byte_len])?;
            // Mirrors Go `m.rand[0] &= uint8(int(1<<msbitLen) - 1)`.
            // When `msbit_len == 8` the Go expression evaluates `(1<<8)-1`
            // = 0xFF; we compute it in u16 space to avoid Rust's shift-overflow
            // panic on `1u8 << 8`, then narrow back to u8.
            let mask = ((1u16 << msbit_len) - 1) as u8;
            self.rand_buf[0] &= mask;

            inc = match byte_len {
                1 => self.rand_buf[0] as u64,
                2 => u16::from_le_bytes([self.rand_buf[0], self.rand_buf[1]]) as u64,
                3 | 4 => {
                    let mut b = [0u8; 4];
                    b[..byte_len].copy_from_slice(&self.rand_buf[..byte_len]);
                    u32::from_le_bytes(b) as u64
                }
                _ => {
                    // 5..=8 — read 8 bytes.
                    let mut b = [0u8; 8];
                    b[..byte_len].copy_from_slice(&self.rand_buf[..byte_len]);
                    u64::from_le_bytes(b)
                }
            };
        }
        // Range: [1, self.inc) per the Go return at line 668.
        Ok(1 + inc)
    }
}

impl<E: Entropy> Entropy for MonotonicEntropy<E> {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        // Plain/io.ReadFull path: behave like the wrapped source
        // when called outside the monotonic-read entry point.
        self.inner.read(dst)
    }
}

impl<E: Entropy> Monotonic for MonotonicEntropy<E> {
    fn monotonic_read(&mut self, ms: u64, dst: &mut [u8]) -> Result<()> {
        self.monotonic_read_inner(ms, dst)
    }
}

/// Thread-safe wrapper around a `Monotonic` source. Mirrors Go
/// `type LockedMonotonicReader struct { mu sync.Mutex; MonotonicReader }`
/// (lines 570-583).
#[cfg(feature = "std")]
pub struct Locked<T: Monotonic> {
    inner: std::sync::Mutex<T>,
}

#[cfg(feature = "std")]
impl<T: Monotonic> Locked<T> {
    pub fn new(inner: T) -> Self {
        Locked {
            inner: std::sync::Mutex::new(inner),
        }
    }

    /// Synchronized monotonic read. Mirrors Go
    /// `func (r *LockedMonotonicReader) MonotonicRead(...) error`
    /// (lines 578-583).
    pub fn monotonic_read(&self, ms: u64, dst: &mut [u8]) -> Result<()> {
        let mut guard = self.inner.lock().expect("monotonic mutex poisoned");
        guard.monotonic_read(ms, dst)
    }
}

#[cfg(feature = "std")]
impl<T: Monotonic> Entropy for Locked<T> {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        let mut guard = self.inner.lock().expect("monotonic mutex poisoned");
        guard.read(dst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entropy::SliceReader;

    /// `uint80` add: plain arithmetic.
    #[test]
    fn uint80_add_basic() {
        let mut u = Uint80 { hi: 0, lo: 0 };
        assert!(!u.add(1));
        assert_eq!(u, Uint80 { hi: 0, lo: 1 });
        // Carry: add u64::MAX into lo=1.
        let _overflow = u.add(u64::MAX);
        assert_eq!(u.lo, 0);
        assert_eq!(u.hi, 1, "Lo carry must increment Hi");
    }

    /// `uint80` overflow on Hi: add a value that pushes Hi past 0xFFFF.
    /// The Go code returns overflow iff `u.Hi < hi` after the Lo-add carry.
    #[test]
    fn uint80_overflow_reports_when_hi_wraps() {
        let mut u = Uint80 { hi: 0xFFFF, lo: 1 };
        // Adding u64::MAX makes Lo wrap to 0, incrementing Hi to 0x10000
        // = 0 in u16 storage. Hi "decreased" 0xFFFF -> 0, so overflow.
        assert!(u.add(u64::MAX), "Hi wrap must report overflow");
    }

    /// `MonotonicEntropy` first call reads from the underlying source;
    /// subsequent same-ms calls increment. Mirrors Go `TestMonotonic`
    /// (lines 531-571) — we use a `SliceReader` feeding deterministic
    /// bytes rather than math/rand, but the relation `next > prev` is
    /// the property under test in both Go and Rust.
    #[test]
    fn monotonic_strictly_increments_same_ms() {
        // Pre-fill with 0xFF entropy and use a single inc == 1 to force
        // minimal growth per increment, mirroring the inc=1 row of
        // the Go parameterised test.
        let payload: Vec<u8> = (0..10).map(|i| (i as u8).wrapping_mul(7)).collect();
        let mut entropy_full = Vec::new();
        entropy_full.extend_from_slice(&payload);
        entropy_full.extend(std::iter::repeat_n(0xAA, 64));
        let mut m = MonotonicEntropy::new(SliceReader::new(&entropy_full), 1);

        let mut prev = [0u8; 10];
        m.monotonic_read(123, &mut prev).unwrap();
        for _ in 0..100 {
            let mut next = [0u8; 10];
            m.monotonic_read(123, &mut next).unwrap();
            assert!(
                next.lexicographically_greater(&prev),
                "non-monotonic prev={prev:?} next={next:?}"
            );
            prev = next;
        }
    }

    /// Mirrors Go `TestMonotonicOverflow` (lines 573-594):
    /// Build a `MonotonicEntropy` whose first read yields 10 bytes of
    /// 0xFF; the second identical-ms call must reject with
    /// `ErrMonotonicOverflow` because adding any positive inc carries
    /// past 2^80 - 1.
    #[test]
    fn monotonic_overflow_returns_sentinel() {
        let ones = vec![0xFFu8; 10];
        let extra: Vec<u8> = (0..64).collect();
        let mut buf = Vec::new();
        buf.extend_from_slice(&ones);
        buf.extend_from_slice(&extra);
        let mut m = MonotonicEntropy::new(SliceReader::new(&buf), 0);

        let mut first = [0u8; 10];
        m.monotonic_read(0, &mut first).unwrap();
        assert_eq!(&first[..], &ones[..]);

        let mut second = [0u8; 10];
        let got = m.monotonic_read(0, &mut second);
        assert_eq!(got, Err(Error::MonotonicOverflow));
    }

    /// Different ms values must reset the entropy — the monotonic state
    /// only applies within the same ms. Mirrors the second-tier
    /// `New`-variant in `TestMonotonic` (the row with timestamps
    /// `[]uint64{122, 123}`, line 667).
    #[test]
    fn monotonic_reset_on_new_ms() {
        // Provide plenty of bytes for two fresh reads.
        let payload: Vec<u8> = (0..32).map(|i| i as u8).collect();
        let mut m = MonotonicEntropy::new(SliceReader::new(&payload), 1);

        let mut a = [0u8; 10];
        m.monotonic_read(122, &mut a).unwrap();
        let mut b = [0u8; 10];
        m.monotonic_read(123, &mut b).unwrap();
        // a and b should differ — the second read pulled the next 10
        // bytes from the source instead of incrementing.
        assert_ne!(a, b);
    }

    /// Convenience trait for the `a > b` byte-wise comparison used above.
    trait LexGreater {
        fn lexicographically_greater(&self, other: &Self) -> bool;
    }
    impl LexGreater for [u8; 10] {
        fn lexicographically_greater(&self, other: &Self) -> bool {
            for (a, b) in self.iter().zip(other.iter()) {
                if a != b {
                    return *a > *b;
                }
            }
            false
        }
    }
}
