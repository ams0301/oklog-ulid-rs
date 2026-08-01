// SPDX-License-Identifier: Apache-2.0
//
// Error variants porting reference/ulid.go lines 50-78.
//
// The Go original uses package-level sentinel `error` values
// (errors.New(...)). The closest idiomatic Rust translation is an enum
// implementing std::error::Error via thiserror-style manual Display/Impl.
// We avoid the `thiserror` crate to keep the port stdlib-only / no_std.
//
// Tests in the Go suite assert `err == io.EOF` after a short read; in the
// port we model that via [`Error::Read(io::ErrorKind::UnexpectedEof)`],
// which the test code matches against `std::io::ErrorKind::UnexpectedEof`.

use core::fmt;

/// Every error the ulid crate can return.
///
/// Each variant corresponds 1:1 to a sentinel error in the Go source,
/// with the addition of [`Error::Read`] which carries the
/// [`std::io::ErrorKind`] returned by a failing entropy source
/// (Go relies on a direct `err == io.EOF` check):
///
/// | Rust variant            | Go sentinel            | ulid.go line |
/// |-------------------------|------------------------|--------------|
/// | `DataSize`              | `ErrDataSize`          | 53           |
/// | `InvalidCharacters`     | `ErrInvalidCharacters` | 57           |
/// | `BufferSize`            | `ErrBufferSize`        | 61           |
/// | `BigTime`               | `ErrBigTime`           | 65           |
/// | `Overflow`              | `ErrOverflow`          | 69           |
/// | `MonotonicOverflow`     | `ErrMonotonicOverflow` | 73           |
/// | `ScanValue`             | `ErrScanValue`         | 77           |
/// | `Read(io::ErrorKind)`   | (`io.EOF` / `io.ErrUnexpectedEOF` etc.) | (entropy calls) |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// `ulid: bad data size when unmarshaling` — wrong length for binary or
    /// text form, or wrong entropy slice length.
    DataSize,
    /// `ulid: bad data characters when unmarshaling` — strict parse saw a
    /// byte that is not part of the Crockford base32 alphabet.
    InvalidCharacters,
    /// `ulid: bad buffer size when marshaling` — destination buffer too
    /// short to receive binary or text form.
    BufferSize,
    /// `ulid: time too big` — construction requested a Unix-ms timestamp
    /// greater than [`crate::MaxTime`].
    BigTime,
    /// `ulid: overflow when unmarshaling` — first base32 char > '7',
    /// i.e. the 130-bit encoded value does not fit in 128 bits.
    Overflow,
    /// `ulid: monotonic entropy overflow` — incrementing the previous
    /// ULID's 80-bit entropy would carry past 2^80 - 1.
    MonotonicOverflow,
    /// `ulid: source value must be a string or byte slice` — `Scan`
    /// received an unsupported dynamic type.
    ScanValue,
    /// An I/O error surfaced from an entropy source.
    ///
    /// Carries the [`std::io::ErrorKind`] so the variant stays `Copy + Eq`
    /// (full `std::io::Error` is neither). The Go suite asserts
    /// `err == io.EOF`; here we match `err == Error::Read(io::ErrorKind::UnexpectedEof)`.
    #[cfg(feature = "std")]
    Read(std::io::ErrorKind),
}

pub type Result<T> = core::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::DataSize => f.write_str("ulid: bad data size when unmarshaling"),
            Error::InvalidCharacters => f.write_str("ulid: bad data characters when unmarshaling"),
            Error::BufferSize => f.write_str("ulid: bad buffer size when marshaling"),
            Error::BigTime => f.write_str("ulid: time too big"),
            Error::Overflow => f.write_str("ulid: overflow when unmarshaling"),
            Error::MonotonicOverflow => f.write_str("ulid: monotonic entropy overflow"),
            Error::ScanValue => f.write_str("ulid: source value must be a string or byte slice"),
            #[cfg(feature = "std")]
            Error::Read(kind) => write!(f, "ulid: entropy read failed: {kind}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}

#[cfg(feature = "std")]
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Read(e.kind())
    }
}

#[cfg(feature = "std")]
impl From<Error> for std::io::Error {
    fn from(e: Error) -> Self {
        match e {
            Error::Read(kind) => std::io::Error::new(kind, e.to_string()),
            other => std::io::Error::new(std::io::ErrorKind::InvalidData, other.to_string()),
        }
    }
}
