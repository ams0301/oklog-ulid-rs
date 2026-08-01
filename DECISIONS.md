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
  Runtime*.
- Small, atomic commits — one logical step per commit — so the work is auditable
  in the order it was produced.

### Porting plan (ordered)
1. Core type `Ulid([u8;16])`, byte layout, `Time`/`SetTime`, `MaxTime`, `Now`/`Timestamp`.
2. Errors module (`ErrDataSize`, `ErrInvalidCharacters`, `ErrBufferSize`, `ErrBigTime`,
   `ErrOverflow`, `ErrMonotonicOverflow`, `ErrScanValue`).
3. Crockford base-32 `dec` table + `parse` (lax + strict) with the unrolled decoder.
4. `MarshalText`/`MarshalTextTo` with the unrolled encoder.
5. `MarshalBinary`/`UnmarshalBinary`, `Bytes`, `Entropy`/`SetEntropy`, `Compare`.
6. `uint80` arithmetic and `MonotonicEntropy` (with `random` from `crypto::rand`
   and a pluggable RNG trait mirroring Go's `io.Reader`).
7. `LockedMonotonicReader` analogue (Rust: `Mutex<MonotonicEntropy>`).
8. `DefaultEntropy`/`Make` (std feature).
9. `cmd/ulid` binary target: `--format`, `--local`, `--quick`, `--zero`.
10. Test port — translating Go `testing/quick.Check`, `iotest.HalfReader`, etc. to Rust
    `proptest` / hand-rolled equivalents, asserting identical behaviour.
11. `criterion` benchmarks, GitHub Actions CI, polish.

### Known adaptation choices (rationale written up here ahead of time)
| Go | Rust | Why |
|---|---|---|
| `io.Reader` entropy source | `Entropy` trait with `fn read(&mut self, dst: &mut [u8]) -> io::Result<()>` | Mirrors `io.ReadFull`; lets us drop in `crypto::rand` or any RNG. |
| `MonotonicReader` interface | `MonotonicEntropy` struct impl + `Locked<T>` wrapper | Go interface → Rust trait feels heavy for 2 impls; simpler with generics. |
| `quick.Check` property tests | `proptest` | Closest Rust equivalent; same semantics (random inputs, shrink on failure). |
| `iotest.HalfReader` | Custom `HalfReader` adapter in `tests/support.rs` | A few lines; not worth a dep. |
| `database/sql` Scan/Value | `serde`-style feature (deferred) | `sqlx`/`diesel` adapters go behind a `sql` feature flag — out of scope unless time allows. |
| `time.Time` | `SystemTime` + `Duration` (std feature) | Direct equivalents; `chrono` avoided as upstream uses only stdlib `time`. |
| `bufio.NewReader` in `Monotonic` | `BufReader`-equivalent layer over the `Entropy` trait | Mirrors Go's intent: amortise reads, allow un-Read `peek`. |

This file is updated on every commit that introduces a non-obvious choice.
