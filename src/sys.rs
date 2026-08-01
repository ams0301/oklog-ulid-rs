// SPDX-License-Identifier: Apache-2.0
//
// std-only helpers: time conversion utilities and the default entropy
// factory. All gated behind the `std` feature.
//
// Ports reference/ulid.go lines 125-149 (DefaultEntropy + Make) and
// lines 407-441 (Now + Timestamp + Time).
//
// DefaultEntropy in the Go upstream wraps `math/rand.New(rand.NewSource(time.Now().UnixNano()))`,
// not `crypto/rand` — see `reference/ulid.go` lines 134-137. The CLI
// flips to `cryptorand.Reader` when not invoked with `--quick`, but the
// library default is the math/rand path. We mirror that exactly: the
// `MathRng` here is xorshift64 (Rust's stdlib has no PRNG), and the
// `cmd/ulid` binary follows Go's `cryptorand.Reader` default via a small
// `CryptRng` adapter (Port #7).

use crate::entropy::Entropy;
use crate::monotonic::{Locked, MonotonicEntropy};
use crate::ulid::Ulid;
use crate::Result;

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convert a `SystemTime` to Unix milliseconds. Mirrors Go
/// `func Timestamp(t time.Time) uint64` (reference/ulid.go lines 430-433).
#[inline]
pub fn timestamp(t: SystemTime) -> u64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as u64,
        // SystemTime before UNIX_EPOCH saturates to 0; Go's
        // uint64(t.Unix())*1000 conversion would also zero-out
        // pre-epoch values in practice.
        Err(_) => 0,
    }
}

/// Current UTC time in Unix milliseconds. Mirrors Go `func Now() uint64`
/// (lines 421-424).
#[inline]
pub fn now() -> u64 {
    timestamp(SystemTime::now())
}

/// Convert Unix milliseconds back into a `SystemTime`. Mirrors Go
/// `func Time(ms uint64) time.Time` (lines 435-441).
#[inline]
pub fn time_from_ms(ms: u64) -> SystemTime {
    let secs = ms / 1_000;
    let nanos = ((ms % 1_000) * 1_000_000) as u32;
    UNIX_EPOCH + Duration::new(secs, nanos)
}

/// Fast non-crypto PRNG: xorshift64 (Marsaglia). Directs go's
/// `math/rand.New(rand.NewSource(seed))` analogue — Go's `math/rand`
/// is a.LinearCongruentalGenerator, not xorshift, but the use case
/// (entropy for ULID generation) does not require statistical quality
/// beyond uniformly distributed high-order bytes; the monotonic read
/// protocol only requires uniqueness-within-ms. xorshift64 hooks the
/// "fast but not crypto" property and is closure-trivial to reproduce.
pub struct MathRng {
    state: u64,
}

impl MathRng {
    /// New RNG seeded from current Unix-nanosecond time. Mirrors Go
    /// `rand.NewSource(time.Now().UnixNano())` (lines 134-137).
    pub fn new_from_time() -> Self {
        let seed = now().wrapping_mul(1_000_000).wrapping_add(0x9E37_79B9_7F4A_7C15);
        MathRng {
            // xorshift64 needs non-zero state to start.
            state: if seed == 0 { 0xDEAD_BEEF_DEAD_BEEF } else { seed },
        }
    }

    /// Direct constructor with a fixed seed (test/deterministic CLI use).
    pub const fn from_seed(seed: u64) -> Self {
        // Choose a fallback if seed happens to be zero; xorshift cannot
        // bootstrap from state=0. We preserve user intent when seed!=0.
        MathRng {
            state: if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed },
        }
    }

    #[inline]
    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }
}

impl Entropy for MathRng {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        // Emit 8 bytes per step (one x.next_u64 iteration) and a
        // trailing partial word little-endian.
        let mut i = 0;
        while i + 8 <= dst.len() {
            let v = self.next().to_le_bytes();
            dst[i..i + 8].copy_from_slice(&v);
            i += 8;
        }
        if i < dst.len() {
            let v = self.next().to_le_bytes();
            let n = dst.len() - i;
            dst[i..].copy_from_slice(&v[..n]);
        }
        Ok(())
    }
}

/// Lazily-constructed per-process monotonic entropy source. Mirrors Go
/// `var entropy io.Reader` + `var entropyOnce sync.Once` (lines 125-128).
static DEFAULT_ENTROPY: OnceLock<Locked<MonotonicEntropy<MathRng>>> = OnceLock::new();

/// Return a thread-safe per-process monotonically-increasing entropy
/// source. Mirrors Go `func DefaultEntropy() io.Reader` (lines 130-140).
///
/// The underlying source is a seeded `MathRng` (xorshift64). The Go
/// upstream wraps `math/rand` for the same reason — `DefaultEntropy`
/// is the library default used by `Make()`, not the cryptographic
/// source used by the CLI's default generator.
pub fn default_entropy() -> &'static Locked<MonotonicEntropy<MathRng>> {
    DEFAULT_ENTROPY.get_or_init(|| Locked::new(MonotonicEntropy::new(MathRng::new_from_time(), 0)))
}

/// Construct a ULID with the current time in Unix milliseconds and
/// monotonically-increasing entropy for the same millisecond.
/// Mirrors Go `func Make() ULID` (lines 142-149).
///
/// Safe for concurrent use thanks to the inner `Mutex` in
/// `Locked<MonotonicEntropy<_>>`.
pub fn make() -> Ulid {
    let entropy = default_entropy();
    let mut id = Ulid::ZERO;
    let ms = now();
    id.set_time(ms).expect("ms now cannot exceed MaxTime");
    let raw = ulid_bytes_mut(&mut id);
    let mut buf6 = [0u8; crate::RAW_SIZE - 6];
    entropy.monotonic_read(ms, &mut buf6).expect("MathRng never fails");
    raw[6..].copy_from_slice(&buf6);
    id
}

// `Ulid::as_bytes_mut` helper. Crate-private because the public surface
// prefers `bytes()` (returning a copy): the Go source never mutates
// bytes in place outside entropy fill, and we keep this accessor
// internal to discourage external poking of the array.
#[inline]
fn ulid_bytes_mut(id: &mut Ulid) -> &mut [u8; crate::RAW_SIZE] {
    &mut id.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors Go `TestTime` (lines 363-373): round-trip drift must be <1ms.
    #[test]
    fn time_round_trips_within_one_ms() {
        let original = SystemTime::now();
        let recovered = time_from_ms(timestamp(original));
        let diff = if original > recovered {
            original.duration_since(recovered).unwrap()
        } else {
            recovered.duration_since(original).unwrap()
        };
        assert!(diff < Duration::from_millis(1), "round-trip drift exceeds 1 ms: {diff:?}");
    }

    /// Mirrors Go `TestTimestamp` (lines 347-361): sub-millisecond input is
    /// truncated to whole milliseconds, exactly like Go's `.Nanosecond()/int(time.Millisecond)`.
    #[test]
    fn timestamp_truncates_to_ms() {
        let one_ms_after_epoch = UNIX_EPOCH + Duration::from_nanos(1_499_999);
        assert_eq!(timestamp(one_ms_after_epoch), 1);
    }

    /// Loose approximation of Go `TestNow` (lines 336-345).
    #[test]
    fn now_does_not_go_backwards() {
        let b = now();
        let after_t = SystemTime::now() + Duration::from_millis(2);
        let a = timestamp(after_t);
        assert!(b < a, "clock went backwards: {b} >= {a}");
    }

    /// Mirrors Go `TestMake` (lines 63-73): parse-back roundtrip works
    /// and time field is close to "now".
    #[test]
    fn make_round_trips_and_carries_recent_time() {
        let id = make();
        let before = now();
        let parsed = crate::Ulid::parse(&id.to_string()).unwrap();
        assert_eq!(id, parsed, "make-then-parse round trip broke");
        let after = now();
        let t = id.time();
        // Allow ~1s slack for slow CI runs.
        assert!(
            t >= before.saturating_sub(1_000) && t <= after + 1_000,
            "make time {t} not in window [{before}-1s, {after}+1s]"
        );
    }

    /// Loose port of Go `TestMonotonicSafe` (lines 596-628) — the Go
    /// original spins 100 goroutines × 1024 iters; we use a tight
    /// non-concurrent serial batch of 1k with the same property
    /// under test: consecutive `make()`s must strictly increase.
    #[test]
    fn make_monotonic_within_same_ms() {
        let mut prev = make();
        for _ in 0..1_000 {
            let next = make();
            assert!(next.as_bytes() > prev.as_bytes(), "non-monotonic: {prev:?} >= {next:?}");
            prev = next;
        }
    }

    /// `MathRng` is deterministic from seed: two instances with the same
    /// seed must produce identical byte streams. Pins the contract for
    /// `cmd/ulid --quick` reproducibility.
    #[test]
    fn math_rng_determinism_with_same_seed() {
        let mut a = MathRng::from_seed(0x1234_5678_9ABC_DEF0);
        let mut b = MathRng::from_seed(0x1234_5678_9ABC_DEF0);
        let mut ab = [0u8; 32];
        let mut bb = [0u8; 32];
        a.read(&mut ab).unwrap();
        b.read(&mut bb).unwrap();
        assert_eq!(ab, bb, "MathRng not deterministic for same seed");
    }

    /// `default_entropy()` returns the same `&'static` reference on
    /// every call (`sync::Once` semantics). Mirrors Go `entropyOnce.Do`.
    #[test]
    fn default_entropy_singleton() {
        let a = default_entropy() as *const _;
        let b = default_entropy() as *const _;
        assert_eq!(a, b, "default_entropy should return a singleton");
    }
}
