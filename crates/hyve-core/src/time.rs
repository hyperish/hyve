//! UTC instants (`hyperish/docs` `03-api-protocol.md` §1: RFC 3339, UTC).

use std::fmt;
use std::str::FromStr;

use crate::platform;

const MICROS_PER_SEC: i64 = 1_000_000;
const SECS_PER_DAY: i64 = 86_400;
/// 0000-01-01T00:00:00Z
const MIN_MICROS: i64 = -62_167_219_200 * MICROS_PER_SEC;
/// 9999-12-31T23:59:59.999999Z
const MAX_MICROS: i64 = 253_402_300_800 * MICROS_PER_SEC - 1;

/// A UTC instant with microsecond precision, limited to years 0000–9999 so it
/// always has an RFC 3339 form.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(i64);

impl Timestamp {
    /// Earliest representable instant (0000-01-01T00:00:00Z).
    pub const MIN: Timestamp = Timestamp(MIN_MICROS);
    /// Latest representable instant (9999-12-31T23:59:59.999999Z).
    pub const MAX: Timestamp = Timestamp(MAX_MICROS);
    /// The Unix epoch.
    pub const UNIX_EPOCH: Timestamp = Timestamp(0);

    /// The current time (clamped into the representable range).
    pub fn now() -> Self {
        Timestamp(platform::now_unix_micros().clamp(MIN_MICROS, MAX_MICROS))
    }

    /// From microseconds since the Unix epoch; `None` outside years 0000–9999.
    pub const fn from_unix_micros(micros: i64) -> Option<Self> {
        if micros < MIN_MICROS || micros > MAX_MICROS {
            None
        } else {
            Some(Timestamp(micros))
        }
    }

    /// From milliseconds since the Unix epoch; `None` outside years 0000–9999.
    pub const fn from_unix_millis(millis: i64) -> Option<Self> {
        match millis.checked_mul(1000) {
            Some(m) => Self::from_unix_micros(m),
            None => None,
        }
    }

    /// Microseconds since the Unix epoch.
    pub const fn as_unix_micros(self) -> i64 {
        self.0
    }

    /// Milliseconds since the Unix epoch (floored).
    pub const fn as_unix_millis(self) -> i64 {
        self.0.div_euclid(1000)
    }

    /// Adds a signed number of microseconds; `None` if the result leaves the range.
    pub fn checked_add_micros(self, micros: i64) -> Option<Self> {
        self.0.checked_add(micros).and_then(Self::from_unix_micros)
    }
}

impl fmt::Debug for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Timestamp({self})")
    }
}

impl fmt::Display for Timestamp {
    /// `YYYY-MM-DDTHH:MM:SS[.fff|.ffffff]Z`; the fraction is omitted when zero and
    /// shortened to milliseconds when the microsecond digits are zero.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let secs = self.0.div_euclid(MICROS_PER_SEC);
        let micros = self.0.rem_euclid(MICROS_PER_SEC);
        let days = secs.div_euclid(SECS_PER_DAY);
        let sod = secs.rem_euclid(SECS_PER_DAY);
        let (y, m, d) = civil_from_days(days);
        write!(
            f,
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
            sod / 3600,
            (sod / 60) % 60,
            sod % 60
        )?;
        if micros != 0 {
            if micros % 1000 == 0 {
                write!(f, ".{:03}", micros / 1000)?;
            } else {
                write!(f, ".{micros:06}")?;
            }
        }
        f.write_str("Z")
    }
}

/// Error returned when text is not an RFC 3339 timestamp in range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseTimestampError(&'static str);

impl fmt::Display for ParseTimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid RFC 3339 timestamp: {}", self.0)
    }
}

impl std::error::Error for ParseTimestampError {}

impl FromStr for Timestamp {
    type Err = ParseTimestampError;

    /// Parses RFC 3339 `date-time` (`T`/`t` separator, `Z`/`z` or `±HH:MM`
    /// offset). Fractions beyond microseconds are truncated; leap seconds (`:60`)
    /// are rejected.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let b = s.as_bytes();
        let err = ParseTimestampError;
        if b.len() < 20 {
            return Err(err("too short"));
        }
        let year = digits(&b[0..4]).ok_or(err("year"))? as i64;
        expect(b, 4, b'-')?;
        let month = digits(&b[5..7]).ok_or(err("month"))? as i64;
        expect(b, 7, b'-')?;
        let day = digits(&b[8..10]).ok_or(err("day"))? as i64;
        if !matches!(b[10], b'T' | b't') {
            return Err(err("expected 'T'"));
        }
        let hour = digits(&b[11..13]).ok_or(err("hour"))? as i64;
        expect(b, 13, b':')?;
        let minute = digits(&b[14..16]).ok_or(err("minute"))? as i64;
        expect(b, 16, b':')?;
        let second = digits(&b[17..19]).ok_or(err("second"))? as i64;
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(err("date out of range"));
        }
        if hour > 23 || minute > 59 || second > 59 {
            return Err(err("time out of range"));
        }

        let mut i = 19;
        let mut micros = 0i64;
        if b[i] == b'.' {
            i += 1;
            let start = i;
            while i < b.len() && b[i].is_ascii_digit() {
                if i - start < 6 {
                    micros = micros * 10 + (b[i] - b'0') as i64;
                }
                i += 1;
            }
            let n = i - start;
            if n == 0 {
                return Err(err("empty fraction"));
            }
            for _ in n..6 {
                micros *= 10;
            }
        }
        let offset_secs = match b.get(i) {
            Some(b'Z' | b'z') if i + 1 == b.len() => 0,
            Some(&sign @ (b'+' | b'-')) if i + 6 == b.len() => {
                let oh = digits(&b[i + 1..i + 3]).ok_or(err("offset hour"))? as i64;
                expect(b, i + 3, b':')?;
                let om = digits(&b[i + 4..i + 6]).ok_or(err("offset minute"))? as i64;
                if oh > 23 || om > 59 {
                    return Err(err("offset out of range"));
                }
                let o = oh * 3600 + om * 60;
                if sign == b'+' { o } else { -o }
            }
            _ => return Err(err("expected 'Z' or a numeric offset")),
        };
        let days = days_from_civil(year, month, day);
        let secs = days * SECS_PER_DAY + hour * 3600 + minute * 60 + second - offset_secs;
        Timestamp::from_unix_micros(secs * MICROS_PER_SEC + micros).ok_or(err("year out of range"))
    }
}

fn expect(b: &[u8], i: usize, c: u8) -> Result<(), ParseTimestampError> {
    if b.get(i) == Some(&c) {
        Ok(())
    } else {
        Err(ParseTimestampError("unexpected separator"))
    }
}

fn digits(b: &[u8]) -> Option<u32> {
    b.iter().try_fold(0u32, |acc, &c| {
        c.is_ascii_digit().then(|| acc * 10 + (c - b'0') as u32)
    })
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn epoch_and_bounds() {
        assert_eq!(Timestamp::UNIX_EPOCH.to_string(), "1970-01-01T00:00:00Z");
        assert_eq!(Timestamp::MIN.to_string(), "0000-01-01T00:00:00Z");
        assert_eq!(Timestamp::MAX.to_string(), "9999-12-31T23:59:59.999999Z");
        assert_eq!(ts("0000-01-01T00:00:00Z"), Timestamp::MIN);
        assert!(Timestamp::from_unix_micros(MAX_MICROS + 1).is_none());
    }

    #[test]
    fn roundtrip_and_fractions() {
        for s in [
            "2026-11-24T12:34:56Z",
            "2026-11-24T12:34:56.123Z",
            "2026-11-24T12:34:56.123456Z",
            "1969-12-31T23:59:59.999999Z",
            "2000-02-29T00:00:00Z",
        ] {
            assert_eq!(ts(s).to_string(), s);
        }
        assert_eq!(
            ts("2026-11-24T12:34:56.1Z").to_string(),
            "2026-11-24T12:34:56.100Z"
        );
        assert_eq!(
            ts("2026-11-24T12:34:56.123456789Z").to_string(),
            "2026-11-24T12:34:56.123456Z"
        );
    }

    #[test]
    fn offsets_convert_to_utc() {
        assert_eq!(ts("2026-11-24T21:00:00+09:00"), ts("2026-11-24T12:00:00Z"));
        assert_eq!(ts("2026-11-24t03:30:00-08:30"), ts("2026-11-24T12:00:00z"));
    }

    #[test]
    fn rejects_invalid() {
        for s in [
            "2026-02-29T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-11-24T24:00:00Z",
            "2026-11-24T23:59:60Z",
            "2026-11-24 12:00:00Z",
            "2026-11-24T12:00:00",
            "2026-11-24T12:00:00.Z",
            "2026-11-24T12:00:00+0900",
            "2026-11-24T12:00:00Zjunk",
        ] {
            assert!(s.parse::<Timestamp>().is_err(), "{s}");
        }
    }

    #[test]
    fn civil_roundtrip_wide_range() {
        let mut d = days_from_civil(0, 1, 1);
        let end = days_from_civil(9999, 12, 31);
        while d <= end {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
            d += 37;
        }
    }

    #[test]
    fn now_is_recent() {
        assert!(Timestamp::now() > ts("2025-01-01T00:00:00Z"));
    }
}
