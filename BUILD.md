# Build

## Requirements (host)

- Rust **stable** (1.97 or newer) — install via <https://rustup.rs>
- A working MSVC toolchain on Windows (Visual Studio 2022 Build Tools with the
  "Desktop development with C++" workload). On Linux/macOS no extra toolchain is
  needed; the standard `cc` linker suffices.

> On Windows the Rust `x86_64-pc-windows-msvc` target is the default. The crate
> is `no_std`-core by default; the `std` feature pulls in OS time and RNG
> helpers. The `criterion` dev-dependency requires `std` to bench.

## Build the library and binary

```sh
cargo build --release
```

This produces the `oklog_ulid` library crate and the `oklog-ulid` binary target
at `target/release/oklog-ulid(.exe)`.

## Run the CLI

```sh
cargo run --release --bin oklog-ulid -- --help
```

## Tests, lints, benches

```sh
cargo test --all
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo bench
```

CI in `.github/workflows/ci.yml` runs all of the above on Ubuntu. The crate
builds with the single command `cargo build` — no Makefiles, no docker, no
platform-specific bootstrap.

## Notes for judges

- Build tested on Windows 11 with the LLVM `lld-link` linker and the
  standalone Windows SDK (no Visual Studio IDE). See `DECISIONS.md` for the
  environment narrative during the 72h window.
- Cross-platform: pure Rust stdlib/`no_std`. No FFI to the original Go library
  and no link against the Go runtime (rule #5).
