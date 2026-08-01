// SPDX-License-Identifier: Apache-2.0
//
// Crockford base-32 codec for ULID text form.
//
// Mirrors reference/ulid.go lines 156-242 (parse / ParseStrict) and
// lines 309-390 (dec table + Encoding alphabet + MarshalTextTo encoder).
//
// ULID text is 26 characters drawn from the alphabet
//   `0123456789ABCDEFGHJKMNPQRSTVWXYZ`
// (Crockford base32, all upper-case: I L O U removed). The decoder is
// case-insensitive — the Go `dec` table maps both the upper- and
// lower-case ASCII positions to the same 5-bit index.

use crate::ulid::RAW_SIZE;
use crate::{Error, Result, ENCODED_SIZE};

/// The Crockford base-32 alphabet used by ULID, in encoding order.
///
/// `0123456789ABCDEFGHJKMNPQRSTVWXYZ` — 32 symbols, missing I/L/O/U.
/// Mirrors Go `const Encoding = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"`.
pub const ENCODING: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Sentinel value indicating "this ASCII byte is not a base-32 char".
///
/// Mirrors Go `0xFF` sentinel used in the `dec` table.
pub const INVALID: u8 = 0xFF;

/// Byte-to-index lookup table for case-insensitive Crockford base-32.
///
/// `DEC[ascii_byte] == INVALID` (0xFF) means the byte is not a valid
/// ULID base-32 character. Otherwise `DEC[ascii_byte]` is the 5-bit
/// symbol value.
///
/// Laid out by indices into a 256-entry table, generated faithfully
/// from the Go `dec` array on lines 363-390 of reference/ulid.go. The
/// original table contains 256 entries; we reproduce them at
/// compile time as a `static` to keep zero-cost lookups.
pub const DEC: [u8; 256] = build_dec();

/// Compile-time construction of the `DEC` lookup table.
///
/// Iterates over ASCII codes 0..=255 and assigns each its Crockford
/// base-32 symbol value, or `0xFF` if invalid. Empirically identical
/// to the Go `dec` array layout.
const fn build_dec() -> [u8; 256] {
    let mut t = [INVALID; 256];
    // Digits 0..=9 -> 0..=9 (ASCII 0x30..=0x39).
    let mut i = 0;
    while i < 10 {
        t[b'0' as usize + i] = i as u8;
        i += 1;
    }
    // Letters A..=Z, skipping I/L/O/U. Indices 10..=31.
    let letters = *b"ABCDEFGHJKMNPQRSTVWXYZ";
    let mut j = 0;
    while j < letters.len() {
        let upper = letters[j] as usize;
        let lower = upper + 32; // ASCII offset to lowercase
        t[upper] = (10 + j) as u8;
        t[lower] = (10 + j) as u8;
        j += 1;
    }
    t
}

/// Decode a 26-character ULID text form into raw bytes.
///
/// Mirrors Go `func parse(v []byte, strict bool, id *ULID) error`
/// (reference/ulid.go lines 171-242). The `strict` flag matches the
/// `ParseStrict` semantics: when true, any byte outside the Crockford
/// alphabet returns [`Error::InvalidCharacters`]; when false, non-base32
/// bytes are passed through the `dec` lookup table producing a
/// `0xFF`::0xFF`-derived but well-defined bit pattern (the Go code calls
/// this "undefined" rather than erroring — we preserve that behaviour).
///
/// `ErrDataSize` is returned when `text.len() != ENCODED_SIZE`.
/// `ErrOverflow` is returned when the first character > '7'
/// (the 130-bit base-32 value would not fit in 128 bits).
pub fn parse(text: &[u8], strict: bool, id: &mut [u8; RAW_SIZE]) -> Result<()> {
    // Check the length first; mirrors Go `if len(v) != EncodedSize`.
    if text.len() != ENCODED_SIZE {
        return Err(Error::DataSize);
    }

    // Strict validation: every byte must be a member of the alphabet.
    // The Go original expands the check over all 26 positions explicitly
    // (lines 179-207); we collapse it to a tight loop with the same
    // runtime cost and identical early-out on the first invalid byte.
    if strict {
        // Unrolled boundary: Go checks `dec[v[0]] == 0xFF || ... || dec[v[25]] == 0xFF`.
        // Equivalent behaviour in Rust: scan and short-circuit. Same result.
        for &b in text {
            if DEC[b as usize] == INVALID {
                return Err(Error::InvalidCharacters);
            }
        }
    }

    // Overflow guard: 128-bit value, so the first base-32 symbol's most
    // significant bit must be 0. Symbol > '7' -> value > 2^128 - 1.
    // Matches Go `if v[0] > '7' { return ErrOverflow }`.
    if text[0] > b'7' {
        return Err(Error::Overflow);
    }

    // Unrolled 130-bit-to-128-bit decoder. Each `dec[v[i]]` is a 5-bit
    // value packed into the 16-byte output. The Go code (lines 222-239)
    // is reproduced verbatim in Rust, byte for byte; the only substitution
    // is `dec[v[i]]` -> `DEC[text[i] as usize]` (array-of-byte indexing
    // has different syntax but identical semantics).
    let v = text;

    // 6 bytes timestamp (48 bits).
    id[0] = (DEC[v[0] as usize] << 5) | DEC[v[1] as usize];
    id[1] = (DEC[v[2] as usize] << 3) | (DEC[v[3] as usize] >> 2);
    id[2] = (DEC[v[3] as usize] << 6) | (DEC[v[4] as usize] << 1) | (DEC[v[5] as usize] >> 4);
    id[3] = (DEC[v[5] as usize] << 4) | (DEC[v[6] as usize] >> 1);
    id[4] = (DEC[v[6] as usize] << 7) | (DEC[v[7] as usize] << 2) | (DEC[v[8] as usize] >> 3);
    id[5] = (DEC[v[8] as usize] << 5) | DEC[v[9] as usize];

    // 10 bytes of entropy (80 bits).
    id[6] = (DEC[v[10] as usize] << 3) | (DEC[v[11] as usize] >> 2);
    id[7] = (DEC[v[11] as usize] << 6) | (DEC[v[12] as usize] << 1) | (DEC[v[13] as usize] >> 4);
    id[8] = (DEC[v[13] as usize] << 4) | (DEC[v[14] as usize] >> 1);
    id[9] = (DEC[v[14] as usize] << 7) | (DEC[v[15] as usize] << 2) | (DEC[v[16] as usize] >> 3);
    id[10] = (DEC[v[16] as usize] << 5) | DEC[v[17] as usize];
    id[11] = (DEC[v[18] as usize] << 3) | (DEC[v[19] as usize] >> 2);
    id[12] = (DEC[v[19] as usize] << 6) | (DEC[v[20] as usize] << 1) | (DEC[v[21] as usize] >> 4);
    id[13] = (DEC[v[21] as usize] << 4) | (DEC[v[22] as usize] >> 1);
    id[14] = (DEC[v[22] as usize] << 7) | (DEC[v[23] as usize] << 2) | (DEC[v[24] as usize] >> 3);
    id[15] = (DEC[v[24] as usize] << 5) | DEC[v[25] as usize];

    Ok(())
}

/// Tests for the base32 decoder. These port the relevant Go cases:
/// TestParseStrictInvalidCharacters (lines 185-213),
/// TestOverflowHandling (lines 483-498),
/// TestParseRobustness (lines 294-334).
#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors Go TestOverflowHandling (lines 483-498).
    ///
    /// Six fixed strings, six expected outcomes. Translates the Go map
    /// iteration into an explicit list to keep a deterministic order.
    #[test]
    fn overflow_handling_matches_go_table() {
        let cases: &[(&[u8], Result<()>)] = &[
            (b"00000000000000000000000000", Ok(())),
            (b"70000000000000000000000000", Ok(())),
            (b"7ZZZZZZZZZZZZZZZZZZZZZZZZZ", Ok(())),
            (b"80000000000000000000000000", Err(Error::Overflow)),
            (b"80000000000000000000000001", Err(Error::Overflow)),
            (b"ZZZZZZZZZZZZZZZZZZZZZZZZZZ", Err(Error::Overflow)),
        ];

        for (input, want) in cases {
            let mut id = [0u8; RAW_SIZE];
            let got = parse(input, false, &mut id);
            assert_eq!(got, *want, "input={:?}", core::str::from_utf8(input));
        }
    }

    /// Mirrors Go TestParseStrictInvalidCharacters (lines 185-213).
    ///
    /// Each of 26 positions, with both 0x00 and 0xFF substitutions, must
    /// be rejected by `ParseStrict` with `ErrInvalidCharacters`.
    #[test]
    fn strict_rejects_invalid_chars_at_every_position() {
        let base = *b"0000XSNJG0MQJHBF4QX1EFD6Y3";
        assert_eq!(base.len(), ENCODED_SIZE);

        for i in 0..ENCODED_SIZE {
            for &bad in &[0x00u8, 0xFFu8] {
                let mut input = base;
                input[i] = bad;
                let mut id = [0u8; RAW_SIZE];
                let got = parse(&input, true, &mut id);
                assert_eq!(
                    got,
                    Err(Error::InvalidCharacters),
                    "position={i} byte=0x{bad:02X}"
                );
            }
        }
    }

    /// Mirrors Go TestParseRobustness (lines 294-334).
    ///
    /// A fixed binary vector and 1e4 random `[u8; 26]` vectors, with the
    /// first byte constrained to <= '7' so the parse can't overflow.
    /// Every parser invocation must succeed (`Parse`, lax mode), per the
    /// Go suite's contract that lax parse never returns ErrInvalidCharacters.
    #[test]
    fn parse_robustness_never_errors_in_lax_mode() {
        // Fixed behavioural vector from Go test (line 297-300).
        let fixed: &[u8] = &[
            0x1, 0xc0, 0x73, 0x62, 0x4a, 0xaf, 0x39, 0x78, 0x51, 0x4e, 0xf8, 0x44, 0x3b, 0xb2,
            0xa8, 0x59, 0xc7, 0x5f, 0xc3, 0xcc, 0x6a, 0xf2, 0x6d, 0x5a, 0xaa, 0x20,
        ];
        let mut id = [0u8; RAW_SIZE];
        parse(fixed, false, &mut id).expect("fixed vector must parse lax-ly");

        // 1e4 random vectors, first byte mod '7' to guaranteed <= '7'.
        let mut seed: u64 = 0xABAD_CAFE_DEAD_BEEF;
        for _ in 0..10_000 {
            let mut s = [0u8; ENCODED_SIZE];
            let mut state = seed;
            for byte in s.iter_mut() {
                // xorshift64
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                *byte = state as u8;
            }
            if s[0] > b'7' {
                s[0] %= b'7';
            }
            let mut id = [0u8; RAW_SIZE];
            parse(&s, false, &mut id).expect("lax parse must never error");
            seed = state;
        }
    }

    /// Lower-case input must round-trip identically to upper-case,
    /// mirroring Go `TestCaseInsensitivity` (lines 277-292). The decoder
    /// must accept both since `DEC` maps both.
    #[test]
    fn decoder_is_case_insensitive() {
        let upper = b"0000XSNJG0MQJHBF4QX1EFD6Y3";
        let lower = b"0000xsnjg0mqjhbf4qx1efd6y3";
        let mut a = [0u8; RAW_SIZE];
        let mut b = [0u8; RAW_SIZE];
        parse(upper, false, &mut a).unwrap();
        parse(lower, false, &mut b).unwrap();
        assert_eq!(a, b, "case-insensitive decode divergence");
    }
}
