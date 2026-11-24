//! Duration literals: one or more `<integer><unit>` groups with strictly
//! decreasing units, no whitespace. Units: `d`, `h`, `m`, `s`, `ms`.

use std::time::Duration;

use hyve_diagnostics::{Diagnostic, Span};
use winnow::ascii::digit1;
use winnow::combinator::alt;
use winnow::prelude::*;

const UNITS: &str = "`d`, `h`, `m`, `s` or `ms`";

/// Microseconds per unit, with the unit's rank (higher = larger).
fn unit(input: &mut &str) -> winnow::Result<(u64, u8)> {
    alt((
        "ms".value((1_000u64, 0u8)),
        "s".value((1_000_000, 1)),
        "m".value((60_000_000, 2)),
        "h".value((3_600_000_000, 3)),
        "d".value((86_400_000_000, 4)),
    ))
    .parse_next(input)
}

/// Parses a duration such as `1h30m`.
///
/// Invalid input is an error, never a silent default (`hyperish/docs`
/// `01-dx-principles.md`): empty text, whitespace, unknown units, repeated or
/// increasing units (`30m1h`), and values that overflow.
pub fn parse_duration(text: &str) -> Result<Duration, Diagnostic> {
    if text.is_empty() {
        return Err(Diagnostic::error("empty duration").with_help(format!(
            "write e.g. `30s`, `5m` or `1h30m` (units: {UNITS})"
        )));
    }
    let mut rest = text;
    let mut total: u64 = 0;
    let mut last_rank: Option<u8> = None;
    while !rest.is_empty() {
        let start = text.len() - rest.len();
        let digits = digit1::<_, winnow::error::ContextError>
            .parse_next(&mut rest)
            .map_err(|_| {
                Diagnostic::error(format!("invalid duration `{text}`"))
                    .with_label(Span::new(start, start + 1), "expected a number")
                    .with_help(format!(
                        "a duration is a number followed by a unit ({UNITS}), e.g. `1h30m`"
                    ))
            })?;
        let unit_start = text.len() - rest.len();
        let (micros, rank) = unit.parse_next(&mut rest).map_err(|_| {
            let end = (unit_start + 1).min(text.len()).max(unit_start);
            Diagnostic::error(format!("invalid duration `{text}`"))
                .with_label(Span::new(unit_start, end), "expected a unit")
                .with_help(format!("use one of {UNITS}"))
        })?;
        let unit_end = text.len() - rest.len();
        if let Some(prev) = last_rank {
            if rank >= prev {
                return Err(Diagnostic::error(format!("invalid duration `{text}`"))
                    .with_label(
                        Span::new(unit_start, unit_end),
                        "units must go from largest to smallest, each at most once",
                    )
                    .with_help("write e.g. `1h30m` instead of `30m1h`"));
            }
        }
        last_rank = Some(rank);
        let overflow = || {
            Diagnostic::error(format!("duration `{text}` is too large"))
                .with_label(Span::new(start, unit_end), "overflows")
        };
        let n: u64 = digits.parse().map_err(|_| overflow())?;
        total = n
            .checked_mul(micros)
            .and_then(|v| total.checked_add(v))
            .ok_or_else(overflow)?;
    }
    Ok(Duration::from_micros(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_forms() {
        assert_eq!(parse_duration("500ms").unwrap(), Duration::from_millis(500));
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("1h30m").unwrap(), Duration::from_secs(5400));
        assert_eq!(
            parse_duration("1d2h3m4s5ms").unwrap(),
            Duration::from_millis(93_784_005)
        );
        assert_eq!(parse_duration("0s").unwrap(), Duration::ZERO);
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            "1",
            "h",
            "1x",
            "1 h",
            "30m1h",
            "1h1h",
            "1.5h",
            "-1s",
            " 1s",
            "1s ",
            "99999999999999999999d",
        ] {
            let e = parse_duration(bad).unwrap_err();
            assert!(
                !e.help().is_empty() || e.message().contains("too large"),
                "{bad}: {e}"
            );
        }
    }

    #[test]
    fn error_points_at_unit() {
        let e = parse_duration("10x").unwrap_err();
        assert_eq!(e.primary_span(), Some(Span::new(2, 3)));
    }
}
