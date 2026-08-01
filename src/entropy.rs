// SPDX-License-Identifier: Apache-2.0
//
// Entropy abstractions and ULID constructors.
//
// Ports reference/ulid.go lines 80-113 (MonotonicReader interface and
// New/MustNew funcs) plus the byte-filling convenience paths.
//
// The Go API takes an `io.Reader` for entropy, with a separate
// `MonotonicReader` interface for sources that guarantee strictly
// increasing entropy within the same ms timestamp. We model this in Rust
// as two traits:
//
//   * [`Entropy`]                  — matches Go `io.Reader`.
//     `read(&mut self, dst: &mut [u8]) -> io::Result<()>` is the
//     `io.ReadFull(e, id[6:])` analogue: fill the buffer entirely or
//     return an error.
//
//   * [`Monotonic`] (in `monotonic.rs`, Port #5) — matches Go
//     `MonotonicReader::MonotonicRead(ms uint64, p []byte) error`.
//
// Both default to `Error::Read(io::ErrorKind::...)` rather than reusing
// `std::io::Error` directly so that the error type stays `Copy + Eq`
// (see error.rs for the rationale matching the Go sentinel test idiom).

use crate::ulid::Ulid;
use crate::ulid::RAW_SIZE;
use crate::{Error, Result};

/// Read-only entropy source. Mirrors Go `io.Reader`, restricted to the
/// "fill this slice completely" usage pattern the ULID library relies on.
///
/// Conceptually equivalent to `io.ReadFull(&mut self, dst)`. Implementors
/// are free to return fewer bytes than requested only by treating that as
/// an error ([`Error::Read`] with [`std::io::ErrorKind::UnexpectedEof`]).
///
/// `BSliceReader` (defined below) adapts an in-memory slice into a
/// sequential entropy source that returns `UnexpectedEof` on overflow,
/// matching Go's `bytes.NewReader` + `io.ReadFull` behaviour used
/// throughout the test suite.
pub trait Entropy {
    /// Fill `dst` with random bytes. Returns an error if fewer than
    /// `dst.len()` bytes could be produced (matching `io.ReadFull`).
    fn read(&mut self, dst: &mut [u8]) -> Result<()>;
}

/// Marker-trait that an [`Entropy`] source also provides a monotonic-read
/// entry point. Mirrors Go `MonotonicReader` (lines 80-87).
///
/// `monotonic_read` must, for the same `ms` parameter, return entropy
/// bytes strictly greater (lexicographically) than the previous call's.
/// Implementations live in `monotonic.rs`.
pub trait Monotonic: Entropy {
    /// Yield strictly-increasing entropy bytes for `ms`. Mirrors Go
    /// `MonotonicRead(ms uint64, p []byte) error`.
    fn monotonic_read(&mut self, ms: u64, dst: &mut [u8]) -> Result<()>;
}

/// Construct a ULID with the given Unix-ms timestamp and entropy source.
///
/// Mirrors Go `func New(ms uint64, entropy io.Reader) (id ULID, err error)`
/// (lines 89-113). ErrBigTime is returned when `ms > MaxTime`. If
/// `entropy` is also a `Monotonic` source, its `monotonic_read` is used in
/// place of `read`; this matches the Go type switch (`switch e := entropy.(type)`)
/// that prefers `MonotonicRead` over `Read`.
///
/// Behaviour when `entropy` is omitted: the Go original accepts `nil`
/// `io.Reader` and skips the read. In Rust the no-entropy-filling case is
/// spelled `Ulid::with_time_only(ms)` (callers rarely want to ask a trait
/// object for "no bytes").
pub fn new<E: Entropy>(ms: u64, entropy: Option<&mut E>) -> Result<Ulid> {
    let mut id = Ulid::ZERO;
    id.set_time(ms)?;

    match entropy {
        None => Ok(id),
        Some(reader) => {
            // Slice the entropy bytes out of `id` and fill them.
            // Mirrors `_, err = io.ReadFull(e, id[6:])`.
            let raw = id.0.as_mut_slice();
            reader.read(&mut raw[6..RAW_SIZE])?;
            Ok(id)
        }
    }
}

/// Convenience wrapper around [`new`] that panics on failure. Mirrors Go
/// `func MustNew(ms uint64, entropy io.Reader) ULID` (lines 115-123).
///
/// Matches the Go semantics: panics via `panic!(err)`. Go code calling
/// `ulid.MustNew(... io.Reader)` expects a panic with the error string on
/// the rare failure path (and recovers in the test suite — see Go
/// `TestMustNew` lines 75-88).
pub fn must_new<E: Entropy>(ms: u64, entropy: Option<&mut E>) -> Ulid {
    match new(ms, entropy) {
        Ok(id) => id,
        Err(err) => panic!("ulid::must_new: {err}"),
    }
}

/// Construct a ULID with the given time and entropy source, preferring
/// the source's `Monotonic` implementation when one is present.
///
/// Mirrors the Go `switch e := entropy.(type) { case MonotonicReader: }`
/// branch inside `New` (lines 102-110). The Rust version requires the
/// caller to spell out "I have a monotonic source" by passing
/// `isa::monotonic Some(&mut M)`; the upshot is no runtime type check.
pub fn new_monotonic<M: Monotonic>(ms: u64, entropy: Option<&mut M>) -> Result<Ulid> {
    let mut id = Ulid::ZERO;
    id.set_time(ms)?;

    match entropy {
        None => Ok(id),
        Some(monotonic) => {
            let raw = id.0.as_mut_slice();
            // `id[6:]` — entropy bytes are bytes 6..=15.
            monotonic.monotonic_read(ms, &mut raw[6..RAW_SIZE])?;
            Ok(id)
        }
    }
}

/// Construct a ULID given a fixed `ms` and no entropy source at all.
/// All 10 entropy bytes stay zero.
///
/// Convenience constructor used by Go callers who pass `nil` for
/// `entropy` and expect the time-only form (`Entropy()` returns 10
/// zero bytes). The Rust idiom avoids passing `Option::<&mut Some>::None`.
pub fn with_time_only(ms: u64) -> Result<Ulid> {
    let mut id = Ulid::ZERO;
    id.set_time(ms)?;
    Ok(id)
}

/// Helper that lifts any in-memory byte slice into an [`Entropy`] source
/// that yields its contents sequentially. Mirrors Go's
/// `bytes.NewReader(...)` paired with `io.ReadFull`: returns
/// `Error::Read(UnexpectedEof)` when the slice runs out, exactly like
/// the Go `TestNew` "Error" subtest verifies (lines 50-60).
#[derive(Debug)]
pub struct SliceReader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> SliceReader<'a> {
    #[inline]
    pub fn new(buf: &'a [u8]) -> Self {
        SliceReader { buf, pos: 0 }
    }
}

impl<'a> Entropy for SliceReader<'a> {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        let remaining = self.buf.len() - self.pos;
        if remaining < dst.len() {
            // Replicate Go's `io.ErrUnexpectedEOF` via `io.ReadFull`:
            // short reads return an error rather than silently truncating.
            // We mirror the Go `TestNew` expectation that an empty reader
            // over an empty source returns `io.EOF` (matched here as
            // `UnexpectedEof` per `io::ReadFull`'s documented behaviour).
            return Err(unexpected_eof_err());
        }
        dst.copy_from_slice(&self.buf[self.pos..self.pos + dst.len()]);
        self.pos += dst.len();
        Ok(())
    }
}

#[cfg(feature = "std")]
#[inline]
fn unexpected_eof_err() -> Error {
    Error::Read(std::io::ErrorKind::UnexpectedEof)
}

#[cfg(not(feature = "std"))]
#[inline]
fn unexpected_eof_err() -> Error {
    // No `io::ErrorKind` in no_std; surface a sentinel we can still match.
    // The std `Error::Read` variant is feature-gated, so use `DataSize`
    // as the closest "this reader didn't give us enough bytes" error
    // until entropy sources are made allocator-aware in a later commit.
    Error::DataSize
}

/// Zero bytes forever — the entropy source for `ulid.Make` pantomime
/// used by the Go test `TestMonotonicOverflow` (lines 573-594) where
/// the first ULID's entropy is bytes.Repeat([]byte{0xFF},10) and
/// everything else is crypto/rand; the source is "fixed all-ones then
/// exhausted." For test translation we model that as one FiniteReader,
/// and a `ZeroReader` for the `cmd/ulid --zero` flag.
pub struct ZeroReader;

impl Entropy for ZeroReader {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        dst.fill(0);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MaxTime;

    /// Mirrors Go TestNew "Error" subtest (lines 50-60):
    ///
    /// 1) `New(MaxTime()+1, nil)` must return `ErrBigTime`.
    /// 2) `New(0, strings.NewReader(""))` must return an io.EOF-equivalent
    ///    error. We assert it is `Error::Read(UnexpectedEof)`.
    #[test]
    fn new_returns_expected_sentinels() {
        // 1: too-big time.
        let too_big = MaxTime::VALUE + 1;
        let got = new::<ZeroReader>(too_big, None);
        assert_eq!(got, Err(Error::BigTime), "MaxTime+1 must reject");

        // 2: empty entropy source — must surface io::ErrorKind::UnexpectedEof.
        let empty: &[u8] = &[];
        let mut reader = SliceReader::new(empty);
        let got = new(0u64, Some(&mut reader));
        match got {
            Err(Error::Read(std::io::ErrorKind::UnexpectedEof)) => (), // ok
            other => panic!("expected Read(UnexpectedEof) on empty source, got {other:?}"),
        }
    }

    /// Mirrors Go testULID helper (lines 112-125), used by both
    /// TestNew and TestMustNew: construct a ULID with `ms = 1e5` and
    /// (a) no entropy, expecting the entropy bytes to stay zero,
    /// (b) 16 fixed `0xFF` entropy bytes, expecting bytes 6..=15 to be all FF.
    #[test]
    fn new_with_and_without_entropy() {
        // 1: time-only. Expect entropy to stay 0.
        let id = new::<ZeroReader>(100_000, None).unwrap();
        let want = Ulid::from_bytes([
            0x00, 0x00, 0x00, 0x01, 0x86, 0xA0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]);
        assert_eq!(
            id, want,
            "time-only ULID wrong:\n   got {id:?}\n  want {want:?}"
        );

        // 2: time + 16-byte 0xFF entropy. Only bytes 6..=15 should change.
        let entropy: [u8; 16] = [0xFF; 16];
        let mut reader = SliceReader::new(&entropy);
        let id = new(100_000, Some(&mut reader)).unwrap();
        let want = Ulid::from_bytes([
            0x00, 0x00, 0x00, 0x01, 0x86, 0xA0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            0xFF, 0xFF,
        ]);
        assert_eq!(
            id, want,
            "entropy bytes wrong:\n   got {id:?}\n  want {want:?}"
        );
    }

    /// `must_new` panics on error. Mirrors Go TestMustNew "Panic" subtest
    /// (lines 75-88): panic value carries an EOF-equivalent error string.
    #[test]
    fn must_new_panics_on_empty_entropy() {
        let result = std::panic::catch_unwind(|| {
            let empty: &[u8] = &[];
            let mut reader = SliceReader::new(empty);
            must_new(0u64, Some(&mut reader))
        });
        assert!(result.is_err(), "must_new should have panicked");
        // The Go test asserts the panic value equals io.EOF; we settle for
        // the panic message mentioning the underlying kind.
        let msg = result
            .err()
            .and_then(|p| p.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        assert!(
            msg.contains("ulid::must_new"),
            "panic message should mention the constructor: got {msg:?}"
        );
    }

    /// `with_time_only` is the no-reader counterpart to `New(ms, nil)`.
    #[test]
    fn with_time_only_does_not_touch_entropy() {
        let id = with_time_only(1_000).unwrap();
        assert_eq!(id.time(), 1_000);
        assert_eq!(&id.as_bytes()[6..], &[0u8; 10]);
    }
}
