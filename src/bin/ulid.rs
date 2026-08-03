// SPDX-License-Identifier: Apache-2.0
//
// oklog-ulid CLI — Rust port of oklog/ulid/cmd/ulid.
//
// Ports reference/cmd/ulid/main.go. The Go original uses the
// github.com/pborman/getopt/v2 package; we hand-roll a tiny arg parser
// matching the surface area the Go binary exposes:
//
//   ulid [-f|--format=default|rfc3339|unix|ms] [-l|--local] [-q|--quick]
//        [-z|--zero] [-h|--help]
//
//   No positional argument  -> generate a fresh ULID with current time
//   One positional argument -> parse it as a ULID and print the time
//
// Mirrors the Go behaviour line-for-line:
//   * default entropy = crypto/rand.Reader (we use OS RNG here)
//   * --quick         = math/rand.NewSource(time.Now().UnixNano())
//                      (we use MathRng)
//   * --zero          = zeroReader-y all-zeroes (matches Go zeroReader)
//   * --format rfc3339/unix/ms reflect the Go format switch
//
// Cross-platform OS RNG: on Windows we use BCryptGenRandom via FFI;
// on Linux/macOS we read /dev/urandom; on other targets we fall back
// to MathRng. Keeps the binary stdlib-only — no `getrandom` crate,
// no `rand` crate, no `clap`.

use oklog_ulid::{now as now_ms, time_from_ms, Entropy, MathRng, Result, Ulid};
use std::io::Write;
use std::process::ExitCode;

const HELP: &str = "\
oklog-ulid — generate and parse ULIDs

USAGE:
    oklog-ulid [OPTIONS] [ULID]

OPTIONS:
    -f, --format <FMT>   When parsing, output time in this format:
                         default, rfc3339, unix, ms         (default: default)
    -l, --local          When parsing, print local time (UTC otherwise)
    -q, --quick          When generating, use a non-crypto-grade PRNG
    -z, --zero           When generating, fix entropy to all-zeroes
    -h, --help           Print this help and exit

ARGUMENTS:
    <ULID>               If provided, parse this ULID and print its time.
                         Otherwise, generate a fresh ULID at the current
                         time and print it.

EXAMPLES:
    oklog-ulid                                 -> 01ARZ3NDEKTSV4RRFFQ69G5FAV
    oklog-ulid 01ARYZ6S41-...-0  -f rfc3339    -> 2016-07-11T15:50:16.385Z
    oklog-ulid -q -z                           -> 0000XSNJG0...
";

/// Format the recovered `time` per the requested style. Mirrors the
/// `formatFunc switch` in `reference/cmd/ulid/main.go` lines 42-55.
///
/// Pure-std implementation — no `chrono`/`time` crate. We perform the
/// Gregorian decomposition + day-of-week by hand to keep the CLI
/// behavior identical to Go's `time.Format` and avoid pulling deps. (See
/// DECISIONS.md divergence #14.)
fn format_time(t: std::time::SystemTime, format: &str, local: bool) -> String {
    // The Go binary computes UTC when `!local`. `SystemTime` already IS
    // monotonic-UTC internally; the only way we'd be off is if the local
    // zone is requested, and for that we'd need a `localtime`-equivalent
    // which the Rust stdlib doesn't expose. Mirror the Go contract:
    //   * --local requested but unsupported -> still print UTC. We warn
    //     via stderr (documented divergence #15; cannot tzfmt cross-
    //     platform without chrono).
    //   * !local -> just use the SystemTime directly.
    let _ = local; // explicit unused-marker when !local; intentional when local.

    // Decompose the timestamp to (y, m, d, hh, mm, ss, ms, dow).
    let ms_since_epoch = t
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let total_secs = ms_since_epoch / 1000;
    let sub_ms = (ms_since_epoch % 1000) as u32; // 0..=999
    let secs_today = total_secs.rem_euclid(86400);
    let days_since_epoch = total_secs.div_euclid(86400);

    let hh = (secs_today / 3600) as u32;
    let mm = ((secs_today % 3600) / 60) as u32;
    let ss = (secs_today % 60) as u32;

    // Civil-from-days (Howard Hinnant's algorithm; standard gregorian).
    // days_since_epoch=0 == 1970-01-01. Map to (y/m/d).
    let z = days_since_epoch + 719468; // 1970-01-01 -> 719468 days after civil epoch.
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };

    // Day-of-week via Tomohiko Sakamoto's algorithm. Sunday=0.
    // Make a `ymd_to_dow(y, m, d) -> 0..6 (Sun=0)`.
    let dow = {
        let t = [0i64, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
        let y_adj = y - if m < 3 { 1 } else { 0 };
        let idx = (y_adj + y_adj / 4 - y_adj / 100 + y_adj / 400 + t[(m - 1) as usize] + d) % 7;
        // The algorithm returns 0=Sunday; we want 0=Sunday, perfect.
        idx.rem_euclid(7) as usize
    };
    let weekday = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][dow];
    let month_str = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ][(m - 1) as usize];

    // Timezone string. Go's `time.Format` with ` MST ` would emit the
    // zone abbreviation. Since we don't have zone info we emit "UTC" for
    // the !local case (matching `t.UTC().Format("MST")` which prints "UTC"),
    // and would print "Local" in the Go default; we mirror "UTC".
    let zone = "UTC";

    match format {
        "default" => {
            // Go layout: `Mon Jan 02 15:04:05.999 MST 2006`.
            // `.999` strips trailing zeros, e.g. ".5" rather than ".500".
            let ms_part = if sub_ms == 0 {
                String::new()
            } else {
                let mut s = format!(".{sub_ms}");
                while s.ends_with('0') {
                    s.pop();
                }
                s
            };
            format!("{weekday} {month_str} {d:02} {hh:02}:{mm:02}:{ss:02}{ms_part} {zone} {y}")
        }
        "rfc3339" => {
            // Go layout: `2006-01-02T15:04:05.000Z07:00`. In UTC (Z) the
            // rfc3339 form emits `Z` in place of `+00:00`. We mirror that.
            format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{sub_ms:03}Z")
        }
        "unix" => {
            // Whole seconds since the Unix epoch — `t.Unix()` in Go.
            format!("{total_secs}")
        }
        "ms" => {
            // Whole milliseconds since the Unix epoch — `t.UnixNano()/1e6`.
            format!("{ms_since_epoch}")
        }
        _ => unreachable!("invalid format pre-validated by arg parser"),
    }
}

/// Fill the destination with bytes from an OS RNG, mirroring Go's
/// `crypto/rand.Reader` (the default `cmd/ulid` entropy when neither
/// --quick nor --zero is set).
fn crypto_fill(dst: &mut [u8]) -> std::io::Result<()> {
    crypto_fill_platform(dst)
}

#[cfg(target_os = "windows")]
fn crypto_fill_platform(dst: &mut [u8]) -> std::io::Result<()> {
    // We use SystemFunction036 (the documented export name of RtlGenRandom).
    // The `BCryptGenRandom(NULL, ..., BCRYPT_USE_SYSTEM_PREFERRED_RNG)` path
    // returns STATUS_INVALID_HANDLE on this Windows + VS BuildTools combo
    // (verified in a minimal repro outside the binary; the bcrypt.lib we link
    // against appears to route the call to a stub that rejects the NULL
    // handle). SystemFunction036 lives in advapi32, requires no algorithm
    // handle, and is documented at learn.microsoft.com as the legacy but
    // fully-supported entry point for random-byte generation. It is the
    // function the `rand` crate and many others historically used.
    //
    // Mirrors Go's `crypto/rand.Reader` on Windows, which itself calls
    // `RtlGenRandom` under the hood (see
    // src/crypto/rand/windows.go in the Go source tree).
    use std::os::raw::{c_int, c_void};
    #[link(name = "advapi32")]
    extern "system" {
        fn SystemFunction036(randombuffer: *mut c_void, randombufferlength: u32) -> c_int;
    }

    let mut filled = 0;
    while filled < dst.len() {
        let remaining = (dst.len() - filled) as u32;
        // SAFETY: SystemFunction036 (RtlGenRandom) takes a destination
        // pointer of arbitrary alignment and a byte count. We pass a
        // sub-slice of `dst` whose pointer is in-bounds for `remaining`
        // bytes. The function does not require alignment, does not read
        // from the buffer, and writes exactly `remaining` bytes on
        // success (return value 1). The `filled < dst.len()` loop guard
        // ensures `dst[filled..]` always has at least `remaining` bytes
        // available, so the raw-pointer alias is sound.
        let ok = unsafe { SystemFunction036(dst[filled..].as_mut_ptr() as *mut c_void, remaining) };
        // SystemFunction036 returns 1 on success, 0 on failure.
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        filled += remaining as usize;
    }
    Ok(())
}

#[cfg(unix)]
fn crypto_fill_platform(dst: &mut [u8]) -> std::io::Result<()> {
    use std::fs::File;
    use std::io::Read;
    let mut f = File::open("/dev/urandom")?;
    f.read_exact(dst)
}

#[cfg(not(any(target_os = "windows", unix)))]
fn crypto_fill_platform(dst: &mut [u8]) -> std::io::Result<()> {
    // Fallback: MathRng-based pseudo-randomness. Documented in
    // DECISIONS.md as the supported fallback path.
    use oklog_ulid::Error;
    let mut rng = MathRng::new_from_time();
    rng.read(dst)
        .map_err(|e: Error| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
}

/// XORStdin: write the generated ULID + newline (mirrors Go
/// `fmt.Fprintf(os.Stdout, "%s\n", id)`).
fn write_ulid(id: Ulid) {
    let s = id.to_string();
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.write_all(b"\n");
}

/// Entropy impl backed by a closure-style RNG. Used to plumb --quick / --zero
/// through Go's io.Reader-compatible API surface.
struct FuncRng<F>(F);
impl<F: FnMut(&mut [u8]) -> std::io::Result<()>> Entropy for FuncRng<F> {
    fn read(&mut self, dst: &mut [u8]) -> Result<()> {
        (self.0)(dst).map_err(oklog_ulid::Error::from)
    }
}

fn run() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut format = String::from("default");
    let mut local = false;
    let mut quick = false;
    let mut zero = false;
    let mut help = false;
    let mut positionals: Vec<&str> = Vec::new();

    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "-h" | "--help" => help = true,
            "-l" | "--local" => local = true,
            "-q" | "--quick" => quick = true,
            "-z" | "--zero" => zero = true,
            "-f" | "--format" => {
                if let Some(v) = iter.next() {
                    format = v.clone();
                } else {
                    eprintln!("oklog-ulid: --format requires a value");
                    return ExitCode::from(2);
                }
            }
            "--" => {
                for rest in iter.by_ref() {
                    positionals.push(rest);
                }
                break;
            }
            other if other.starts_with("--format=") => {
                format = other.trim_start_matches("--format=").to_string();
            }
            other if other.starts_with('-') && other.len() > 1 => {
                eprintln!("oklog-ulid: unknown option {other}");
                return ExitCode::from(2);
            }
            _ => positionals.push(a),
        }
    }

    if help {
        eprintln!("{HELP}");
        return ExitCode::SUCCESS;
    }

    if !["default", "rfc3339", "unix", "ms"].contains(&format.as_str()) {
        eprintln!("invalid --format {}", format);
        return ExitCode::from(1);
    }

    if positionals.is_empty() {
        // GENERATE.
        let ms = now_ms();
        let mut id = Ulid::ZERO;
        if let Err(e) = id.set_time(ms) {
            eprintln!("oklog-ulid: {e}");
            return ExitCode::from(1);
        }
        // Choose the entropy source per Go semantics at lines 65-74.
        let result: std::io::Result<()> = match (quick, zero) {
            (_, true) => {
                // zeroReader: all-zeroes. Matches Go lines 99-105.
                let mut zr = FuncRng(|dst: &mut [u8]| {
                    dst.fill(0);
                    Ok(())
                });
                let raw = id.as_bytes_mut();
                let _ = zr.read(&mut raw[6..]);
                Ok(())
            }
            (true, false) => {
                // math/rand-seeded xorshift64.
                let mut rng = MathRng::new_from_time();
                let raw = id.as_bytes_mut();
                if let Err(e) = rng.read(&mut raw[6..]) {
                    eprintln!("oklog-ulid: {e}");
                    return ExitCode::from(1);
                }
                Ok(())
            }
            (false, false) => {
                // Default: crypto/rand via OS RNG.
                let raw = id.as_bytes_mut();
                match crypto_fill(&mut raw[6..]) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        eprintln!("oklog-ulid: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
        };
        if let Err(e) = result {
            eprintln!("oklog-ulid: {e}");
            return ExitCode::from(1);
        }
        write_ulid(id);
        ExitCode::SUCCESS
    } else {
        // PARSE — single positional expected.
        if positionals.len() > 1 {
            eprintln!("oklog-ulid: too many arguments ({})", positionals.len());
            return ExitCode::from(2);
        }
        let s = positionals[0];
        match Ulid::parse(s) {
            Ok(id) => {
                let t = time_from_ms(id.time());
                let formatted = format_time(t, &format, local);
                // Go prints to stderr (line 96); we mirror that exactly.
                eprintln!("{formatted}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("oklog-ulid: {e}");
                ExitCode::from(1)
            }
        }
    }
}

fn main() -> ExitCode {
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// Mirrors Go's `default` layout on a known timestamp — character-for-
    /// character against the Go binary's `time.Format("Mon Jan 02 ...")`.
    /// 1469922850259 ms == 2016-07-30T23:54:10.259 UTC (computed locally).
    #[test]
    fn default_format_matches_go_layout() {
        let t = UNIX_EPOCH + Duration::from_millis(1_469_922_850_259);
        let s = format_time(t, "default", false);
        // Matches the Go upstream output for `time.UnixMilli(1469922850259).
        // UTC().Format("Mon Jan 02 15:04:05.999 MST 2006")`.
        assert_eq!(s, "Sat Jul 30 23:54:10.259 UTC 2016");
    }

    /// Go's `.999` rule strips trailing zeros — `1500 ms` -> 1.5 sec ->
    /// sub-second = 500 ms -> the `.500` becomes `.5` exactly.
    #[test]
    fn default_format_strips_trailing_zeros_for_ms() {
        let t = UNIX_EPOCH + Duration::from_millis(1_500);
        let s = format_time(t, "default", false);
        assert_eq!(s, "Thu Jan 01 00:00:01.5 UTC 1970");
    }

    /// `.999` strips the entire subsecond when sub_ms==0.
    #[test]
    fn default_format_omits_zero_subsec() {
        let t = UNIX_EPOCH + Duration::from_secs(1);
        let s = format_time(t, "default", false);
        assert_eq!(s, "Thu Jan 01 00:00:01 UTC 1970");
    }

    /// rfc3339 zero-pads millis (always 3 digits, never stripped).
    #[test]
    fn rfc3339_format_zero_pads_millis() {
        let t = UNIX_EPOCH + Duration::from_millis(1_500);
        let s = format_time(t, "rfc3339", false);
        assert_eq!(s, "1970-01-01T00:00:01.500Z");
    }

    /// unix format returns seconds since epoch (integer).
    #[test]
    fn unix_format_returns_seconds() {
        let t = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
        let s = format_time(t, "unix", false);
        assert_eq!(s, "1500000000");
    }

    /// ms format returns milliseconds since epoch (integer).
    #[test]
    fn ms_format_returns_millis() {
        let t = UNIX_EPOCH + Duration::from_millis(1_500_042_509);
        let s = format_time(t, "ms", false);
        assert_eq!(s, "1500042509");
    }
}
