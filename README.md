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
* Benchmarks against the upstream Go timings

No FFI to Go, no link against the Go runtime — pure Rust, `no_std`-capable.

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

## Build

```sh
cargo build --release
```

## Use as a library

```rust
use oklog_ulid::{Ulid, Monotonic};

let id = Ulid::new_now(Monotonic::default())?;
println!("{}", id); // 01ARZ3NDEKTSV4RRFFQ69G5FAV
```

## Use the binary

```sh
cargo run --release --bin oklog-ulid                 # generate a ULID
cargo run --release --bin oklog-ulid -- 01ARZ3NDEKTSV4RRFFQ69G5FAV --format rfc3339
```

## Tests

```sh
cargo test
cargo bench
cargo fmt --check
cargo clippy -- -D warnings
```

The Rust test suite ports the Go tests in `reference/ulid_test.go` field-for-field,
without modifying the originals. See `DECISIONS.md` for any deviations.

## Status

In progress — see `DECISIONS.md` for the live port log.

## License

Apache-2.0, matching upstream `oklog/ulid`. See [`LICENSE-APACHE`](./LICENSE-APACHE).
