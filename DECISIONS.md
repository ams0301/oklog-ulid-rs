# DECISIONS.md — Port Log

A running log of every non-trivial decision, deviation, and trade-off made during the
72-hour Port-Mortem port of `oklog/ulid` (Go → Rust).

Original rule: **Tests are the North Star** — original test suite is hashed at kickoff
and the originals are not modified. This file documents *why* the Rust port makes the
choices it makes, and any place where the Rust side diverges in spirit from the Go side.

---

## T-0 — Kickoff

### Target
- **Track:** E (Go → Rust)
- **Repo:** `oklog/ulid` — https://github.com/oklog/ulid
- **Tag:** `v2.1.0`
- **Pin:** `09b4b3eae8826fac0fcc4d1505eb00179d508cf6`

### Frozen artefacts
- `reference/ulid.go` — original library source (untouched)
- `reference/ulid_test.go` — original test suite (untouched)
- `reference/cmd/ulid/main.go` — original CLI source (untouched)
- `ORIGINAL_TESTS.sha256` — SHA-256 of `ulid_test.go`, captured at kickoff:
  ```
  9deea937b0836e0213271bed4efd25d0caf2d7e0dada0a7f987d53893fe79027  ulid_test.go
  ```

### License
Apache-2.0, matching upstream. Upstream copyright: `2016 The Oklog Authors`.

### Porting philosophy
- Verbatim API surface wherever Rust idiom permits (same names, same semantics,
  same error variants, same byte layout).
- `no_std` first; the `std` feature adds `DefaultEntropy`, time helpers, and
  stdlib IO adapters.
- No link against Go, no FFI into the original library. Rule #5: *No Source-Language
  Runtime*. Host-OS FFI (BCryptGenRandom / SystemFunction036 / /dev/urandom) is used
  by the CLI for crypto-grade RNG, exactly as Go's `crypto/rand` does on each host.
- Small, atomic commits — one logical step per commit — so the work is auditable
  in the order it was produced.

---

## Per-port decisions

### Port #1 — Core `Ulid` type, `Error`, `MaxTime` (commit `b7244e0`)

- `Ulid(pub(crate) [u8;16])` — field is `pub(crate)` so internal modules can touch
  the bytes directly without leaking the layout to consumers. Public API is the
  named accessor methods (`as_bytes`, `as_bytes_mut`, `from_bytes`, `to_bytes`),
  matching the Go `id[:]` slice access pattern.
- `Error` enum carries an `ErrorKind` for the entropy-read variant: keeps the
  type `Copy + Eq` so the Go-style `err == io.EOF` equality check still works.
- `set_time(ms)` returns `Result<()>` mapping `ErrBigTime`. Tests assert via
  `assert_eq!(err, Error::BigTime)` — exactly the Go `ErrBigTime` sentinel shape.

### Port #2 — Crockford base32 decoder (commit `386c397`)

- `DEC[256]` built by a `const fn build_dec()` rather than a hard-coded table.
  The byte sequence is identical to Go's `dec` table — just produced at compile
  time so the source stays readable.
- `parse()` unrolled to sixteen 5-bit slots matching Go lines 222-239. The
  explicit indexing keeps the binary-level byte assignment visible, which is
  required for the strict-lexicographic-order test (`TestLexicographicalOrder`).

### Port #3 — Marshal / Display / binary round-trips (commit `cce2803`)

- `marshal_text_to` is the unrolled encoder, mirror of Go `MarshalTextTo`. The
  public path is `impl Display for Ulid` which gives `to_string()` for free under
  std — matching Go's `String()` method.
- `bytes()` returns an owned `[u8; 16]` (a copy), not a `&[u8]`. Rust idiom plus
  the Go test `TestULID_Bytes` (which asserts mutating the returned slice leaves
  the source unchanged) make the by-value return both idiomatic and correct by
  construction.
- `unmarshal_binary` rejects any length other than 16 with `Error::DataSize`,
  mirroring Go's `ErrDataSize`.

### Port #4 — Entropy abstraction + new/must_new (commit `865c3f1`)

- `Entropy` trait mirrors `io.Reader` restricted to fill-the-buffer semantics
  (read_exact-style). Single method `read(&mut [u8]) -> Result<()>`.
- `Monotonic` trait thunks on top of `Entropy` with `monotonic_read(ms, dst)`,
  mirroring Go's `MonotonicReader` interface.
- `must_new` panics with the `Display`-formatted `Error`, matching Go
  `Must(…)`'s panicking behaviour recoverable via `defer/recover()`.

### Port #5 — Uint80 + MonotonicEntropy + Locked<T> (commit `865c3f1`)

- `Uint80{hi:u16, lo:u64}` with `add`, `is_zero`, `set_bytes`, `append_to`.
  Mirrors Go's `type uint80 struct { Hi, Lo uint64 }` — Rust uses `u16` for Hi
  since `Hi` only ever holds the top 16 bits, catching the overflow flag
  naturally rather than masking. Same wire format and arithmetic.
- `MonotonicEntropy<E: Entropy>` implements the Go `MonotonicEntropy` semantics:
  same-ms increment, new-ms reset, overflow sentinel on `1<<80` wrap.
- `Locked<T: Monotonic>` is `Mutex<T>` + `Monotonic` impl. Go's
  `LockedMonotonicReader` becomes `Locked<T>` — same idea, generics-friendly.

### Port #6 — std helpers: now/timestamp/make/MathRng/default_entropy (commit `4694ee9`)

- `MathRng` (xorshift64) stands in for Go's `math/rand.NewSource(time.Now().UnixNano())`.
  Faithful to the upstream's choice of `math/rand` (not `crypto/rand`) for
  `DefaultEntropy`. The CLI's `--quick` flag uses the same RNG.
- `default_entropy()` returns a `&'static Locked<MonotonicEntropy<MathRng>>` via
  `sync::OnceLock`. Matches Go's `entropyOnce.Do(func() { ... })`.
- `make()` calls `default_entropy().monotonic_read(ms, dst)` with the current
  `now()`. Mirrors Go `func Make() ULID` line-for-line.

### Port #7 — CLI binary `oklog-ulid`, OS RNG (commit `37e7630`)

- Hand-rolled argv parser. The Go original uses `pborman/getopt/v2`; we needed
  no transitive dep so wrote a tight loop matching that flag surface
  (`-f|--format`, `-l|--local`, `-q|--quick`, `-z|--zero`, `-h|--help`).
- OS RNG on Windows: `SystemFunction036` (the documented export name of
  `RtlGenRandom`) from `advapi32`. Initially tried `BCryptGenRandom` with
  `NULL` algorithm handle and `BCRYPT_USE_SYSTEM_PREFERRED_RNG` flag — that
  returned `STATUS_INVALID_HANDLE` (0xC000_0008) consistently on this Windows
  + VS BuildTools combo, verified with a minimal out-of-tree repro. Go's
  `crypto/rand` on Windows itself uses `RtlGenRandom`, so the host-RNG FFI
  chosen here matches the upstream behaviour exactly without invoking Go.
- On Unix `/dev/urandom` is read directly; on other targets `MathRng` is the
  documented fallback.
- New `Ulid::as_bytes_mut()` public method: mirrors the Go `id[6:]` write
  path. Needed because `pub(crate)` field-access doesn't cross the lib/bin
  crate boundary (the binary is a separate downstream crate).

### Port #8 — Compare / MustParse / MustParseStrict + integration tests (commit `3a66ca0`)

- `Ulid::compare(&Ulid) -> Ordering`: thin named alias over the derived `Ord`.
  Go's `func (id ULID) Compare(other ULID) int` delegates to `bytes.Compare`,
  which on big-endian bytes equals numeric compare = `Ord::cmp` here.
- `Ulid::must_parse` and `must_parse_strict`: panic with the Display-formatted
  `Error` so `catch_unwind` yields a stringifiable payload (Go recovers the
  error value itself; we recover a formatted string — same spirit, adapted to
  Rust panic semantics).
- `tests/integration.rs` lives in `tests/` so it compiles as a downstream
  crate, exercising only the published API. Each test carries a doc-comment
  cross-referencing the upstream Go test it mirrors.

### Port #9 — README and DECISIONS polish (this commit)

- README reflects the actual API surface (`make()`, `Ulid::parse`, `time()`)
  rather than placeholder names, and documents the OS-RNG host-FFI choice
  explicitly so reviewers see why `advapi32` is linked.
- No benchmark suite was added; `criterion` was dropped early in the port to
  keep the disk footprint small on the build host. The Rust test suite tests
  behaviour and correctness; performance parity with Go is documented as future
  work to avoid leaving a half-baked bench harness in the tree.
- `database/sql` Scan/Value (Go `TestScan`) is intentionally not ported:
  Rust's SQL ecosystem is split across `sqlx` / `diesel` / `rusqlite`, and
  porting to any one would force a transitive dependency on a specific driver.
  `Error::Scan` is retained in the enum so callers can build their own
  adapter if they wish.

---

## Known adaptation choices (consolidated)

| Go | Rust | Why |
|---|---|---|
| `io.Reader` entropy source | `Entropy` trait with `fn read(&mut [u8]) -> Result<()>` | Mirrors `io.ReadFull`; lets us drop in `crypto/rand` or any RNG. |
| `MonotonicReader` interface | `MonotonicEntropy<E>` struct + `Locked<T>` wrapper | Go interface → Rust trait feels heavy for 2 impls; simpler with generics. |
| `quick.Check` property tests | Hand-rolled census vectors | `proptest` would add a transitive dep; deterministic member-spanning vectors cover the cases the Go suite used. |
| `iotest.HalfReader` | Custom `SliceReader` over a half-length buffer | A few lines; not worth a dep. |
| `database/sql` Scan/Value | `Error::Scan` retained, no DB impl | Avoids dragging in `sqlx`/`diesel`/`rusqlite`. Adapter is the consumer's call. |
| `time.Time` | `SystemTime` + `Duration` (std feature) | Direct equivalents; `chrono` avoided since upstream uses only stdlib `time`. |
| `bufio.NewReader` in `Monotonic` | `MonotonicEntropy<E>` owns the `Entropy` source | Same amortisation effect; simpler lifetime story than wrapping a BufReader around a trait. |
| `math/rand.NewSource` (Go default entropy) | `MathRng` xorshift64 | Faithful: Go's `DefaultEntropy` deliberately uses `math/rand`, not `crypto/rand`. `--quick` CLI flag exercises the same RNG. |
| `crypto/rand.Reader` (CLI default) | OS RNG: `SystemFunction036` / `/dev/urandom` / `MathRng` fallback | No `getrandom` dep; Go's `crypto/rand` itself calls `RtlGenRandom` on Windows. |

This file is updated on every commit that introduces a non-obvious choice.
