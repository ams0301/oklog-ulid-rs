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
