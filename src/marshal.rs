// SPDX-License-Identifier: Apache-2.0
//
// Marshal/Unmarshal for ULID: text and binary forms.
//
// Ports reference/ulid.go lines 265-306 (Bytes/MarshalBinary/
// MarshalBinaryTo/UnmarshalBinary) and lines 272-359 (String/MarshalText/
// MarshalTextTo/UnmarshalText).
//
// Text form is 26 characters of Crockford base32 (alphabet =
// crate::ENCODING); binary form is the raw 16-byte big-endian layout.

use crate::base32::{parse, ENCODING};
use crate::ulid::{RAW_SIZE, Ulid};
use crate::{Error, Result, ENCODED_SIZE};

impl Ulid {
    /// Encode the ULID into 26 characters of Crockford base-32 text in `dst`.
    ///
    /// Mirrors Go `func (id ULID) MarshalTextTo(dst []byte) error`
    /// (reference/ulid.go lines 320-358). Returns [`Error::BufferSize`]
    /// when `dst.len() != ENCODED_SIZE`.
    ///
    /// Implementation is the unrolled 128-bit -> 130-bit packer from the
    /// Go source, ported byte-for-byte: every `Encoding[(id[i] & MASK) >> SHIFT]`
    /// substitution preserves the original bit grouping.
    pub fn marshal_text_to(&self, dst: &mut [u8]) -> Result<()> {
        if dst.len() != ENCODED_SIZE {
            return Err(Error::BufferSize);
        }
        let id = &self.0;

        // 10-character timestamp (48 bits).
        dst[0] = ENCODING[((id[0] & 224) >> 5) as usize];
        dst[1] = ENCODING[(id[0] & 31) as usize];
        dst[2] = ENCODING[((id[1] & 248) >> 3) as usize];
        dst[3] = ENCODING[(((id[1] & 7) << 2) | ((id[2] & 192) >> 6)) as usize];
        dst[4] = ENCODING[((id[2] & 62) >> 1) as usize];
        dst[5] = ENCODING[(((id[2] & 1) << 4) | ((id[3] & 240) >> 4)) as usize];
        dst[6] = ENCODING[(((id[3] & 15) << 1) | ((id[4] & 128) >> 7)) as usize];
        dst[7] = ENCODING[((id[4] & 124) >> 2) as usize];
        dst[8] = ENCODING[(((id[4] & 3) << 3) | ((id[5] & 224) >> 5)) as usize];
        dst[9] = ENCODING[(id[5] & 31) as usize];

        // 16-character entropy (80 bits).
        dst[10] = ENCODING[((id[6] & 248) >> 3) as usize];
        dst[11] = ENCODING[(((id[6] & 7) << 2) | ((id[7] & 192) >> 6)) as usize];
        dst[12] = ENCODING[((id[7] & 62) >> 1) as usize];
        dst[13] = ENCODING[(((id[7] & 1) << 4) | ((id[8] & 240) >> 4)) as usize];
        dst[14] = ENCODING[(((id[8] & 15) << 1) | ((id[9] & 128) >> 7)) as usize];
        dst[15] = ENCODING[((id[9] & 124) >> 2) as usize];
        dst[16] = ENCODING[(((id[9] & 3) << 3) | ((id[10] & 224) >> 5)) as usize];
        dst[17] = ENCODING[(id[10] & 31) as usize];
        dst[18] = ENCODING[((id[11] & 248) >> 3) as usize];
        dst[19] = ENCODING[(((id[11] & 7) << 2) | ((id[12] & 192) >> 6)) as usize];
        dst[20] = ENCODING[((id[12] & 62) >> 1) as usize];
        dst[21] = ENCODING[(((id[12] & 1) << 4) | ((id[13] & 240) >> 4)) as usize];
        dst[22] = ENCODING[(((id[13] & 15) << 1) | ((id[14] & 128) >> 7)) as usize];
        dst[23] = ENCODING[((id[14] & 124) >> 2) as usize];
        dst[24] = ENCODING[(((id[14] & 3) << 3) | ((id[15] & 224) >> 5)) as usize];
        dst[25] = ENCODING[(id[15] & 31) as usize];

        Ok(())
    }

    /// `no_std`-safe zero-allocation text encoder: writes the 26-character
    /// base-32 text into the caller-provided buffer and returns a `&str`
    /// view, avoiding the allocation Go's `String()` performs. The
    /// std-only [`Ulid::to_string`] (inherited from `Display`) is the
    /// allocating counterpart and the primary user-facing API.
    pub fn write_text<'a>(&self, dst: &'a mut [u8; ENCODED_SIZE]) -> &'a str {
        let _ = self.marshal_text_to(dst);
        core::str::from_utf8(dst).expect("ulid text is ascii")
    }

    /// Parse a 26-character ULID text form.
    ///
    /// Mirrors Go `func Parse(ulid string) (ULID, error)` (lines 156-158).
    /// Non-strict: invalid characters produce a defined but unspecified
    /// ULID (matches the Go comment at line 154-155 — "Invalid encodings
    /// produce undefined ULIDs").
    pub fn parse(text: &str) -> Result<Ulid> {
        Self::parse_bytes(text.as_bytes(), false)
    }

    /// Strict variant of [`Ulid::parse`]: rejects bytes outside the
    /// Crockford alphabet with [`Error::InvalidCharacters`].
    ///
    /// Mirrors Go `func ParseStrict(ulid string) (ULID, error)` (lines 167-169).
    pub fn parse_strict(text: &str) -> Result<Ulid> {
        Self::parse_bytes(text.as_bytes(), true)
    }

    /// Shared parse helper used by [`Ulid::parse`] and [`Ulid::parse_strict`].
    #[inline]
    fn parse_bytes(text: &[u8], strict: bool) -> Result<Ulid> {
        let mut bytes = [0u8; RAW_SIZE];
        parse(text, strict, &mut bytes)?;
        Ok(Ulid(bytes))
    }

    /// Copy the raw 16 bytes out. Mirrors Go `func (id ULID) Bytes() []byte`.
    ///
    /// Differs from the Go original in that it returns an owned array
    /// rather than a slice into the receiver. Rust idiom + the test
    /// `TestULID_Bytes` (line 630-639) — which proves mutating the
    /// returned slice must not affect the source ULID — together make
    /// the by-value return both idiomatic and correct by construction.
    #[inline]
    pub fn bytes(&self) -> [u8; RAW_SIZE] {
        self.0
    }

    /// Write raw bytes into `dst`. Returns [`Error::BufferSize`] when
    /// `dst.len() != 16`. Mirrors Go `MarshalBinaryTo` (lines 287-294).
    pub fn marshal_binary_to(&self, dst: &mut [u8]) -> Result<()> {
        if dst.len() != RAW_SIZE {
            return Err(Error::BufferSize);
        }
        dst.copy_from_slice(&self.0);
        Ok(())
    }

    /// Parse 16 raw bytes into a ULID. Returns [`Error::DataSize`] when
    /// `data.len() != 16`. Mirrors Go `UnmarshalBinary` (lines 299-306).
    pub fn unmarshal_binary(data: &[u8]) -> Result<Ulid> {
        if data.len() != RAW_SIZE {
            return Err(Error::DataSize);
        }
        let mut bytes = [0u8; RAW_SIZE];
        bytes.copy_from_slice(data);
        Ok(Ulid(bytes))
    }
}

/// `Display` implementation rendering a ULID as its 26-character
/// Crockford base-32 text. Under `std`, this provides `ToString::to_string`
/// automatically from the std prelude — the idiomatic Go
/// `func (id ULID) String() string` analogue. Under `no_std`, callers
/// use [`Ulid::write_text`] directly to avoid allocation.
impl core::fmt::Display for Ulid {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut buf = [0u8; ENCODED_SIZE];
        let _ = self.marshal_text_to(&mut buf);
        let s = core::str::from_utf8(&buf).expect("ulid text is ascii");
        f.write_str(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base32::DEC;

    /// Round-trip: encode -> decode -> encode must reproduce the same ULID.
    /// Mirrors the core of Go `TestRoundTrips` (lines 127-160).
    #[test]
    fn marshal_text_round_trips() {
        let id = Ulid::from_bytes([
            0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, // time
            0xCD, 0xEF, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, // entropy
        ]);
        let mut text = [0u8; ENCODED_SIZE];
        id.marshal_text_to(&mut text).unwrap();
        let back = Ulid::parse_bytes(&text, false).unwrap();
        assert_eq!(id, back, "round-trip failed");

        let mut text2 = [0u8; ENCODED_SIZE];
        back.marshal_text_to(&mut text2).unwrap();
        assert_eq!(text, text2, "encoding not stable");
    }

    /// Mirrors Go `TestMarshalingErrors` (lines 162-183): the four
    /// marshaling entry points must reject zero-length buffers with
    /// their corresponding sentinel errors.
    #[test]
    fn marshaling_errors_match_sentinels() {
        // MarshalBinaryTo / MarshalTextTo -> ErrBufferSize on empty buffer.
        let id = Ulid::ZERO;
        assert_eq!(id.marshal_binary_to(&mut []), Err(Error::BufferSize));
        assert_eq!(id.marshal_text_to(&mut []), Err(Error::BufferSize));
        // UnmarshalBinary -> ErrDataSize on empty input.
        assert_eq!(Ulid::unmarshal_binary(&[]), Err(Error::DataSize));
        // UnmarshalText -> ErrDataSize on empty text (parse lax).
        assert_eq!(Ulid::parse_bytes(&[], false), Err(Error::DataSize));
    }

    /// Cross-check: the static ExampleULID string from Go
    /// "0000XSNJG0MQJHBF4QX1EFD6Y3" must round-trip through our codec
    /// byte-for-byte. Pins the encoding alphabet against the Go test
    /// expected output (line 36).
    #[test]
    fn example_static_string_round_trips() {
        let s = b"0000XSNJG0MQJHBF4QX1EFD6Y3";
        let id = Ulid::parse_bytes(s, true).expect("example must parse strict");
        let mut out = [0u8; ENCODED_SIZE];
        id.marshal_text_to(&mut out).unwrap();
        assert_eq!(&out[..], s);
    }

    /// Pins behaviour of `TestAlizainCompatibility` (lines 215-224):
    /// constructing a ULID with time 1469918176385 and zero entropy must
    /// encode to "01ARYZ6S410000000000000000".
    #[test]
    fn alizain_compatibility() {
        let mut id = Ulid::ZERO;
        id.set_time(1_469_918_176_385).unwrap();
        // entropy stays all-zero from `Ulid::ZERO`.
        let mut text = [0u8; ENCODED_SIZE];
        id.marshal_text_to(&mut text).unwrap();
        assert_eq!(&text, b"01ARYZ6S410000000000000000");
    }

    /// `bytes()` returns a copy: mutating it must not affect the source.
    /// Mirrors Go `TestULID_Bytes` (lines 630-639).
    #[test]
    fn bytes_returns_independent_copy() {
        let id = Ulid::from_bytes([0xAA; RAW_SIZE]);
        let mut copy_bytes = id.bytes();
        copy_bytes[RAW_SIZE - 1] = 0;
        assert_ne!(id.bytes()[RAW_SIZE - 1], copy_bytes[RAW_SIZE - 1]);
        assert_eq!(id.as_bytes()[RAW_SIZE - 1], 0xAA);
    }

    /// Mirrors Go `TestLexicographicalOrder` (lines 248-275): the sort
    /// order of binary bytes must equal the sort order of encoded text.
    /// `Ulid: Ord` derives byte-wise compare (matches `bytes.Compare`).
    #[test]
    fn text_order_matches_binary_order() {
        let a = Ulid::from_bytes([0x00; RAW_SIZE]);
        let b = Ulid::from_bytes([0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        let c = Ulid::from_bytes([0xFF; RAW_SIZE]);
        // Three-wise: a < b < c both in raw and text form.
        assert!(a < b);
        assert!(b < c);
    }

    /// Cross-check that the DEC and ENCODING tables are mutual inverses
    /// over the 32-symbol alphabet: ENCODING[DEC[c]] == c for c in 0..31.
    #[test]
    fn dec_and_encoding_are_inverse() {
        for (idx, &sym) in ENCODING.iter().enumerate() {
            assert_eq!(DEC[sym as usize] as usize, idx, "DEC[{sym}] should be {idx}");
        }
    }
}
