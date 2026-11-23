//! The canary's window (V11-22): the fleet's side of it, cut from the bots' exports.
//!
//! The venue's docs do not give an export hand's shape. The fields read here are the ones v10
//! read all season: `started_at` (RFC 3339), `profit` (our net chips) and `big_blind`. Nothing is
//! defaulted: a hand missing one is not counted, and is counted as unread, so a fleet short of its
//! hands is refused as not yet measured, never measured on a guess.

use crate::store::ExportLine;

/// The fleet's side of the canary's window.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fleet {
    /// What each hand started in the window won, in big blinds.
    pub results: Vec<f64>,
    /// Hands that could not be placed in the window, or were in it with no result to read.
    pub unread: usize,
}

/// The hands of `lines` started from `from_ms` up to `until_ms`, both included.
pub fn fleet(lines: &[ExportLine], from_ms: i64, until_ms: i64) -> Fleet {
    let mut fleet = Fleet::default();
    for line in lines {
        let Some(started) = line.started_at.as_deref().and_then(started_ms) else {
            fleet.unread += 1;
            continue;
        };
        if !(from_ms..=until_ms).contains(&started) {
            continue;
        }
        match (line.profit, line.big_blind) {
            (Some(profit), Some(big_blind)) if big_blind > 0.0 => {
                fleet.results.push(profit / big_blind);
            }
            _ => fleet.unread += 1,
        }
    }
    fleet
}

/// An RFC 3339 time as milliseconds since 1970: a date, `T` (or a space), a time to the second
/// with any fraction of one, and `Z` or an offset. A time with no offset is the venue's own, UTC.
pub fn started_ms(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't' | b' ')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let field = |from: usize, to: usize| digits(text.get(from..to)?);
    let (year, month, day) = (field(0, 4)?, field(5, 7)?, field(8, 10)?);
    let (hour, minute, second) = (field(11, 13)?, field(14, 16)?, field(17, 19)?);
    if !(1..=12).contains(&month)
        || !(1..=days_in(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut rest = &text[19..];
    let mut millis = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let end = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if end == 0 {
            return None;
        }
        // The first three digits are the milliseconds, padded with zeros; the rest are finer.
        millis = fraction
            .bytes()
            .take(end.min(3))
            .chain(std::iter::repeat(b'0'))
            .take(3)
            .fold(0, |ms, digit| ms * 10 + i64::from(digit - b'0'));
        rest = &fraction[end..];
    }
    let offset = match rest {
        "" | "Z" | "z" => 0,
        _ => {
            let sign = match rest.as_bytes().first()? {
                b'+' => 1,
                b'-' => -1,
                _ => return None,
            };
            if rest.len() != 6 || rest.as_bytes()[3] != b':' {
                return None;
            }
            let (hours, minutes) = (digits(&rest[1..3])?, digits(&rest[4..6])?);
            if hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 60 + minutes)
        }
    };
    let seconds = days_since_1970(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second
        - offset * 60;
    Some(seconds * 1_000 + millis)
}

/// A field of decimal digits, and nothing else.
fn digits(text: &str) -> Option<i64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn days_in(year: i64, month: i64) -> i64 {
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The days from 1970-01-01 to a date of the proleptic Gregorian calendar (Howard Hinnant's
/// `days_from_civil`): years are counted from March, so the leap day falls last.
fn days_since_1970(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    era * 146_097 + of_cycle - 719_468
}
