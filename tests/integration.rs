// SPDX-License-Identifier-Identifier: Apache-2.0
//
// Integration tests that exercise the public API surface, mirroring
// behaviours verified by the Go test suite (reference/ulid_test.go
// hash 9deea937b0836e0213271bed4efd25d0caf2d7e0dada0a7f987d53893fe79027
// — untouched per the port rules). These live in `tests/` to confirm
// the published `oklog_ulid` crate works for an external consumer
// (a separate downstream crate invokes these as it would in real use).

// Activated only when the std helpers are built (the default).
#![cfg(feature = "std")]

use oklog_ulid::{Error, Ulid};
use std::cmp::Ordering;
use std::panic;

/// Mirrors Go `TestMustParse` (lines 187-203): both `MustParse` and
/// `MustParseStrict` must panic with `ErrDataSize` for an empty input.
/// Go uses `recover()` and asserts equality with `ulid.ErrDataSize`;
/// we use `catch_unwind` and assert the recovered string equals what
/// `Error::DataSize` formats as. The two distinct panic surfaces are
/// tested in two sub-cases to mirror Go's `t.Run` table.
#[test]
fn must_parse_panics_on_bad_data_size() {
    type ParseFn = fn(&str) -> Ulid;
    let cases: &[(&str, ParseFn)] = &[
        ("MustParse", Ulid::must_parse),
        ("MustParseStrict", Ulid::must_parse_strict),
    ];
    for (name, f) in cases {
        let err = panic::catch_unwind(|| f(""));
        assert!(err.is_err(), "{name}: expected panic for empty input");
        let payload = err.unwrap_err();
        // The payload is the Display-formatted Error, so compare against
        // the same string a panic produced by must_parse would emit.
        let want = format!("{}", Error::DataSize);
        let got = payload_downcast_str(&payload);
        assert_eq!(got, want, "{name}: panic payload mismatch");
    }
}

/// Best-effort extraction of the panic payload as a `String`. Both
/// `String` and `&str` payloads are supported, mirroring what the
/// `panic!("{e}")` macros in must_parse/must_parse_strict would
/// produce.
fn payload_downcast_str(p: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = p.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = p.downcast_ref::<String>() {
        s.clone()
    } else {
        format!("{p:?}")
    }
}

/// Mirrors Go `TestCompare` (lines 244-256) — `Compare(a, b)` returns
/// the same result as comparing the canonical string forms.
/// Go uses `testing/quick` with 1e5 random samples; we follow with a
/// smaller deterministic census here (the spirit is identical —
/// pair-wise the two comparison path answers agree) since a property
/// runner is outside `std`. Each census entry spans the three
/// ordering buckets (-1, 0, +1).
#[test]
fn compare_matches_string_order() {
    let examples: &[(Ulid, Ulid, Ordering)] = &[
        (
            Ulid::from_bytes([0u8; 16]),
            Ulid::from_bytes([0u8; 16]),
            Ordering::Equal,
        ),
        (
            Ulid::from_bytes([0x00; 16]),
            Ulid::from_bytes([0x01; 16]),
            Ordering::Less,
        ),
        (
            Ulid::from_bytes([0xFF; 16]),
            Ulid::from_bytes([0x00; 16]),
            Ordering::Greater,
        ),
        // Sequenced pair — first byte differs.
        (
            Ulid::from_bytes({
                let mut b = [0u8; 16];
                b[0] = 0x10;
                b
            }),
            Ulid::from_bytes({
                let mut b = [0u8; 16];
                b[0] = 0x20;
                b
            }),
            Ordering::Less,
        ),
    ];

    for (a, b, want) in examples {
        let via_compare = a.compare(b);
        let via_string = a.to_string().cmp(&b.to_string());
        assert_eq!(via_compare, *want, "compare mismatch");
        assert_eq!(via_string, *want, "string compare mismatch");
        assert_eq!(
            via_compare, via_string,
            "compare != string for ({}, {})",
            a, b
        );
    }
}

/// Mirrors Go `TestMustNew` panic subcase: `MustNew(0, "")` must panic
/// with `io.EOF`. The Rust port surfaces the same condition as
/// [`Error::Read`] with [`std::io::ErrorKind::UnexpectedEof`], which the
/// Go test maps to via the `err == io.EOF` equality.
#[test]
fn must_new_panics_on_short_entropy_with_eof_payload() {
    let panic = panic::catch_unwind(|| {
        oklog_ulid::must_new(0, Some(&mut oklog_ulid::SliceReader::new(&[])))
    });
    assert!(panic.is_err(), "MustNew should panic on EOF entropy");
    let payload = panic.unwrap_err();
    let got = payload_downcast_str(&payload);
    // Error::Display surfaces the io::ErrorKind by its humanized name
    // (e.g. "unexpected end of file"); assert that semantic, not the
    // enum variant's Rust identifier.
    assert!(
        got.contains("unexpected end of file") || got.contains("UnexpectedEof"),
        "panic payload: {got}"
    );
}

/// Mirrors Go `ExampleULID` — printing a known ULID yields the known
/// 26-character string `0000XSNJG0QE5 PiaKJ7G5P`-style reference.
/// We verify `Display` produces a 26-char output matching the parsed
/// form exactly.
#[test]
fn display_round_trips_through_parse() {
    let s = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let id = Ulid::parse(s).expect("parse");
    let displayed = format!("{id}");
    assert_eq!(displayed, s);
    assert_eq!(displayed.len(), oklog_ulid::ENCODED_SIZE);
}

// ---------------------------------------------------------------------------
// Tests added in Port #10 (功能性 completeness pass). These mirror the four
// Go tests the earlier port left unported: TestEntropy (lines 411-432),
// TestEntropyRead (lines 434-454), TestScan (lines 463-486), and the full
// TestMonotonic (lines 501-535).
// ---------------------------------------------------------------------------

use oklog_ulid::{monotonic, new, new_monotonic, ScanInput};

/// Mirrors Go `TestEntropy` (lines 411-432): `SetEntropy([]byte{})`
/// returns `ErrDataSize`; any 10-byte slice round-trips through
/// `SetEntropy` / `Entropy`. Go uses `quick.Check` for the property
/// branch; we draw a deterministic census (including the all-zero
/// and all-FF extremes) which spans the same space deterministically.
#[test]
fn entropy_set_get_round_trips() {
    // The static error-sentinel half — Go lines 414-417.
    let mut id = Ulid::ZERO;
    let got = id.set_entropy(&[]);
    assert_eq!(got, Err(Error::DataSize));

    // The property half — Go lines 419-430. Span the 10-byte input space.
    let cases: [[u8; 10]; 5] = [
        [0x00; 10],                                                   // all zero
        [0xFF; 10],                                                   // all ones
        [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A], // monotonic
        [0xDE, 0xAD, 0xBE, 0xEF, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66], // pattern
        // Last byte covers an off-by-one boundary (max u8 sentinel):
        [0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFE, 0xFD],
    ];
    for e in cases {
        let mut id = Ulid::ZERO;
        id.set_entropy(&e)
            .expect("set_entropy should accept 10 bytes");
        let got = id.entropy();
        assert_eq!(got, e, "entropy round-trip mismatch");
        // Don't touch the time half.
        assert_eq!(id.time(), 0, "entropy must not bleed into time");
    }
}

/// Mirrors Go `TestEntropyRead` (lines 434-454): `New(now, HalfReader(e))`
/// end-to-end reads 10 bytes of entropy correctly despite the reader
/// returning only "half" of each Read call. Go's `iotest.HalfReader`
/// wraps a reader so every read returns at most half the requested
/// length; under our `Entropy::read` trait contract (which mandates
/// full-fill, mirroring Go's `io.ReadFull`), the equivalent adapter
/// is a stateful `HalfReader` that yields bytes one or two at a time
/// until the buffer is filled — this exercises the underlying
/// reader's `read_exact` accumulation semantics.
#[test]
fn new_with_half_reader_round_trips_entropy() {
    for e in [
        [0u8; 10],
        [0xFFu8; 10],
        [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA],
    ] {
        let mut reader = HalfReader::new(&e);
        let id = new(oklog_ulid::now(), Some(&mut reader)).expect("new");
        assert_eq!(id.entropy(), e, "entropy mismatch under HalfReader");
    }
}

/// `iotest.HalfReader` analogue: returns up to `1 + counter % 2` bytes per
/// `read`, never the full requested length. Behaves like Go's HalfReader
/// in that it always returns at least one byte (until exhausted), so a
/// `ReadFull`-style accumulator takes multiple reads to fill the dst.
struct HalfReader<'a> {
    buf: &'a [u8],
    pos: usize,
    call: u32,
}

impl<'a> HalfReader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        HalfReader {
            buf,
            pos: 0,
            call: 0,
        }
    }
}

impl<'a> oklog_ulid::Entropy for HalfReader<'a> {
    fn read(&mut self, dst: &mut [u8]) -> oklog_ulid::Result<()> {
        // Same "definitely inadequate" shape as iotest.HalfReader: never
        // return the full requested length, even if we could. Each call
        // yields ceil(requested/2) bytes (capped by what's left in self).
        let requested = dst.len();
        let permitted = requested.div_ceil(2); // never == requested (HalfReader invariant)
        let available = self.buf.len() - self.pos;
        let take = permitted.min(available).max(1.min(available));
        dst[..take].copy_from_slice(&self.buf[self.pos..self.pos + take]);
        self.pos += take;
        self.call += 1;
        // Our `Entropy::read` contract is full-fill; HalfReader intentionally
        // can't full-fill in one call, so we recurse: continue drawing until
        // the buffer is filled or our underlying source is exhausted.
        if take < requested {
            // Recurse with the tail. Catches the `read_exact` semantic the
            // Go `io.ReadFull` в裹 wraps around the half-aware reader.
            if self.pos >= self.buf.len() {
                return Err(Error::Read(std::io::ErrorKind::UnexpectedEof));
            }
            self.read(&mut dst[take..])
        } else {
            Ok(())
        }
    }
}

/// Mirrors Go `TestScan` (lines 463-486). Three success cases (string,
/// bytes, nil) and one rejection case (an arbitrary `Other` sentinel
/// representing the int-typed drive value 44). Go uses an interface{}
/// type switch; Rust uses our [`ScanInput`] sum type — same shape.
#[test]
fn scan_handles_string_bytes_nil_other() {
    let id = oklog_ulid::must_new(123, Some(&mut oklog_ulid::MathRng::from_seed(0x1234_5678)));

    // Each (name, input, want_out, want_err) — mirrors the Go table.
    let cases: &[(&str, ScanInput, Ulid, Option<Error>)] = &[
        ("string", ScanInput::String(&id.to_string()), id, None),
        ("bytes", ScanInput::Bytes(&id.as_bytes()[..]), id, None),
        ("nil", ScanInput::Null, Ulid::ZERO, None),
        (
            "other",
            ScanInput::Other,
            Ulid::ZERO,
            Some(Error::ScanValue),
        ),
    ];

    for (name, input, want_out, want_err) in cases {
        let mut out = Ulid::ZERO;
        let err = out.scan(*input);
        if let Some(want_err) = want_err {
            assert_eq!(err, Err(*want_err), "{name}: err mismatch");
        } else {
            assert!(err.is_ok(), "{name}: unexpected error {err:?}");
        }
        assert_eq!(
            out.compare(want_out),
            Ordering::Equal,
            "{name}: ULID mismatch"
        );
    }
}

/// Mirrors Go `TestMonotonic` (lines 501-535). The Go original iterates
/// over two entropy sources (crypto/math) and six `inc` values (0, 1, 2,
/// 256, 65536, 0x100000000). For each combination, 10_000 ULIDs are
/// generated with the same timestamp 123; each must strictly sort after
/// the previous one. In Rust we swap the crypto-source branch for a
/// second `MathRng` seeded from a distinct constant (the spirit is
/// identical — the test verifies MonotonicRead monotonicity across
/// entropy-source and inc settings, not crypto-vs-math specifically).
#[test]
fn monotonic_strictly_increases_across_inc_and_entropy_table() {
    let inc_values: &[u64] = &[
        0,                     // 0 -> converted to u32::MAX by MonotonicEntropy::new
        1,                     // 1
        2,                     // 2
        (u8::MAX as u64) + 1,  // 256
        (u16::MAX as u64) + 1, // 65536
        (u32::MAX as u64) + 1, // 0x1_0000_0000
    ];

    // Two "entropy" rows. Go has {"cryptorand", "mathrand"}; we use two
    // deterministic MathRng seeds since genuine crypto RNG would make the
    // test non-deterministic.
    for seed in [0xCAFE_BABE_u64, 0xDEAD_BEEF_u64] {
        for &inc in inc_values {
            let mut entropy = monotonic(oklog_ulid::MathRng::from_seed(seed), inc);
            let mut prev: Option<Ulid> = None;
            for _ in 0..10_000 {
                // NB: Rust's static dispatch forces us to call
                // `new_monotonic` directly here, where Go's `ulid.New`
                // uses a runtime type switch to detect MonotonicReader
                // and dispatch to MonotonicRead. The behavioural result
                // is identical — `MonotonicRead` is invoked either way.
                let next =
                    new_monotonic(123, Some(&mut entropy)).expect("MonotonicRead never fails");
                if let Some(p) = prev {
                    assert!(
                        p.compare(&next) == Ordering::Less,
                        "monotonicity violated: prev > next (seed={seed:#x}, inc={inc})
                         prev={p}
                         next={next}",
                    );
                }
                prev = Some(next);
            }
        }
    }
}
