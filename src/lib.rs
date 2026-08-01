// SPDX-License-Identifier: Apache-2.0
//
// Rust port of oklog/ulid (Go) — Track E: Go -> Rust
//
// Port-Mortem Code Resurrection 2026, Wave 2 — Hackathon Raptors
//
// Source pin: oklog/ulid @ v2.1.0
//   commit 09b4b3eae8826fac0fcc4d1505eb00179d508cf6
//
// An ULID is a 16-byte Universally Unique Lexicographically Sortable
// Identifier with a 48-bit Unix-millisecond time prefix and 80 bits of
// entropy, base32-encoded into a 26-character string.
//
// Layout (mirrors reference/ulid.go lines 30-47):
//
//   0                   1                   2                   3
//   0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                      32_bit_uint_time_high                    |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |     16_bit_uint_time_low      |       16_bit_uint_random      |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                       32_bit_uint_random                      |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
//  |                       32_bit_uint_random                      |
//  +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
extern crate std;

mod base32;
mod entropy;
mod error;
mod marshal;
mod monotonic;
mod ulid;

pub use base32::{parse, DEC, ENCODING, INVALID};
pub use entropy::{
    must_new, new, new_monotonic, with_time_only, Entropy, Monotonic, SliceReader, ZeroReader,
};
pub use error::{Error, Result};
#[cfg(feature = "std")]
pub use monotonic::Locked;
pub use monotonic::{MonotonicEntropy, Uint80};
pub use ulid::{MaxTime, Ulid, ENCODED_SIZE, RAW_SIZE};

#[cfg(feature = "std")]
mod sys;

#[cfg(feature = "std")]
pub use sys::{default_entropy, make, now, time_from_ms, timestamp, MathRng};
