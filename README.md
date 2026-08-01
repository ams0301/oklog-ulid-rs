# oklog-ulid-rs

A faithful **Rust** port of [`oklog/ulid`](https://github.com/oklog/ulid) (Go), written for
**Hackathon Raptors — Port-Mortem Code Resurrection 2026, Wave 2**,
**Track E (Go → Rust)**.

`oklog/ulid` implements [ULID](https://github.com/ulid/spec): Universally Unique
Lexicographically Sortable Identifiers — 128-bit, 26-character Crockford-base32
strings whose sort order matches the time-prefixed binary layout.

This port preserves:

* The full public API surface of `oklog/ulid v2.1.0`
* The binary layout (16 bytes, MSB-first, 48-bit ms timestamp + 80-bit entropy)
* The Crockford base-32 alphabet (`0123456789ABCDEFGHJKMNPQRSTVWXYZ`)
* Monotonic entropy semantics with `uint80` overflow detection
* The `oklog/ulid` binary (`cmd/ulid`) as a Rust binary target

No FFI to Go, no link against the Go runtime — pure Rust, `no_std`-capable.
Host-OS FFI is used only for the CLI's default crypto-grade RNG
(`SystemFunction036`/`RtlGenRandom` on Windows, `/dev/urandom` on Unix), which
matches what Go's `crypto/rand` uses under the hood on each host.

## Source pin

The upstream repository is pinned for verifiability:

```
repo    github.com/oklog/ulid
tag     v2.1.0
commit  09b4b3eae8826fac0fcc4d1505eb00179d508cf6
```

A frozen copy of the original sources (untouched) lives in [`reference/`](./reference).
The SHA-256 of the original test file is recorded in
[`ORIGINAL_TESTS.sha256`](./ORIGINAL_TESTS.sha256) and was captured at kickoff.
Original tests are not modified (North Star rule).

## Build

```sh
cargo build --release            # builds the lib + the oklog-ulid binary
```

Single build command, as required by the competition rules. No external crate
dependencies — the whole port lives off `std` (and a `no_std` minimal core for
the library module).

## Use as a library

```rust
use oklog_ulid::{Ulid, make};

// Generate a fresh monotonic ULID at the current time, using the
// crate's thread-safe default entropy source (MathRng-backed
// MonotonicEntropy behind a sync::OnceLock + Mutex).
let id: Ulid = make();
println!("{id}");   // e.g. 01ARZ3NDEKTSV4RRFFQ69G5FAV

// Parse and read its time:
let parsed = Ulid::parse("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
let ms = parsed.time();   // 1469922850259
```

## Use the binary

```sh
cargo run --release --bin oklog-ulid                            # generate
cargo run --release --bin oklog-ulid -- -q                      # MathRng (--quick)
cargo run --release --bin oklog-ulid -- -z                      # zero entropy
cargo run --release --bin oklog-ulid -- 01ARZ3NDEKTSV4RRFFQ69G5FAV -f ms   # parse -> ms
cargo run --release --bin oklog-ulid -- 01ARZ3NDEKTSV4RRFFQ69G5FAV -f unix # parse -> secs
cargo run --release --bin oklog-ulid -- -h                      # help
```

Flags map 1:1 to the Go CLI surface: `-f|--format`, `-l|--local`, `-q|--quick`,
`-z|--zero`, `-h|--help`. Exit codes: 0 on success, 1 on ULID error, 2 on
argument error.

## Tests

```sh
cargo test                       # 34 tests (30 lib + 4 integration)
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

The Rust test suite ports the Go tests in `reference/ulid_test.go` behaviour-for-behaviour,
without modifying the originals. Each `#[test]` carries a doc comment naming the
upstream Go test it mirrors (e.g. `TestMustParse`, `TestCompare`, `TestMonotonicSafe`).
See `DECISIONS.md` for documented deviations.

## Crate layout

```
src/
  lib.rs          crate root, module wiring, public re-exports
  ulid.rs         Ulid([u8;16]), MaxTime, time/set_time, as_bytes[_mut], compare
  error.rs        Error enum (sentinel variants, Copy + Eq), Result
  base32.rs       DEC table, ENCODING alphabet, parse() unrolled decoder
  marshal.rs      marshal_text_to encoder, parse/parse_strict, must_parse[_strict],
                  marshal_binary/unmarshal_binary, Display impl
  entropy.rs      Entropy / Monotonic traits, new/must_new, new_monotonic,
                  with_time_only, SliceReader, ZeroReader
  monotonic.rs    Uint80, MonotonicEntropy<E>, Locked<T> (std mutex wrapper)
  sys.rs          std-only: timestamp/now/time_from_ms, MathRng xorshift64 PRNG,
                  default_entropy() OnceLock singleton, make()
  bin/ulid.rs     CLI binary — hand-rolled argv parser + OS RNG

tests/
  integration.rs  public-API integration tests (TestMustParse, TestCompare,
                  TestMustNew panic, Display round-trip)
```

## License

Apache-2.0, matching upstream `oklog/ulid`. See [`LICENSE-APACHE`](./LICENSE-APACHE).
