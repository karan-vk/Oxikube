//! The date column of an `ls -l` line, in the four shapes the supported `ls` programs print.

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::{Offset, TimeZone};

use super::token;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// Consumes the date from the start of `rest`. `None` when it is not a date this parser knows;
/// otherwise `Some(seconds since the epoch)`.
pub(super) fn take(rest: &mut &str, now: i64) -> Option<Option<i64>> {
    let mut probe = *rest;
    let first = token(&mut probe)?;
    let seconds = if is_iso_date(first) {
        iso(first, &mut probe)?
    } else if WEEKDAYS.contains(&first) {
        ctime(&mut probe)?
    } else {
        classic(first, &mut probe, now)?
    };
    *rest = probe;
    Some(Some(seconds))
}

fn is_iso_date(word: &str) -> bool {
    let bytes = word.as_bytes();
    bytes.len() == 10 && bytes[4] == b'-' && bytes[7] == b'-'
}

fn number(word: &str) -> Option<i64> {
    if word.is_empty() || !word.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    word.parse().ok()
}

fn month_number(word: &str) -> Option<i8> {
    let at = MONTHS.iter().position(|month| *month == word)?;
    i8::try_from(at + 1).ok()
}

/// A day: year, month, day of the month.
type Day = (i64, i8, i64);
/// A time of day: hour, minute, second, nanosecond.
type Clock = (i64, i64, i64, i32);

fn at((year, month, day): Day, (hour, minute, second, nano): Clock, zone: TimeZone) -> Option<i64> {
    let civil = DateTime::new(
        i16::try_from(year).ok()?,
        month,
        i8::try_from(day).ok()?,
        i8::try_from(hour).ok()?,
        i8::try_from(minute).ok()?,
        i8::try_from(second).ok()?,
        nano,
    )
    .ok()?;
    Some(civil.to_zoned(zone).ok()?.timestamp().as_second())
}

/// `2024-03-05 12:34:56.123456789 +0000` (GNU and busybox `--full-time`) or `2024-03-05 12:34`
/// (GNU with an ISO `TIME_STYLE`).
fn iso(date: &str, rest: &mut &str) -> Option<i64> {
    let year = number(&date[0..4])?;
    let month = i8::try_from(number(&date[5..7])?).ok()?;
    let day = number(&date[8..10])?;
    let time = token(rest)?;
    let mut parts = time.split(':');
    let hour = number(parts.next()?)?;
    let minute = number(parts.next()?)?;
    let Some(seconds_part) = parts.next() else {
        return at((year, month, day), (hour, minute, 0, 0), TimeZone::UTC);
    };
    let (whole, fraction) = seconds_part.split_once('.').unwrap_or((seconds_part, ""));
    let second = number(whole)?;
    let nano = i32::try_from(number(&format!("{fraction:0<9}")[..9]).unwrap_or(0)).ok()?;
    let zone = token(rest)?;
    let offset = offset_seconds(zone)?;
    let zone = TimeZone::fixed(Offset::from_seconds(offset).ok()?);
    at((year, month, day), (hour, minute, second, nano), zone)
}

/// `+0100`, `-0530`.
fn offset_seconds(word: &str) -> Option<i32> {
    let sign = match word.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let digits = &word[1..];
    if digits.len() != 4 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = digits[2..].parse().ok()?;
    Some(sign * (hours * 3600 + minutes * 60))
}

/// `Mar  5 12:34:56 2024` after the weekday (busybox `ls -e`).
fn ctime(rest: &mut &str) -> Option<i64> {
    let month = month_number(token(rest)?)?;
    let day = number(token(rest)?)?;
    let time = token(rest)?;
    let year = number(token(rest)?)?;
    let mut parts = time.split(':');
    let hour = number(parts.next()?)?;
    let minute = number(parts.next()?)?;
    let second = number(parts.next()?)?;
    at((year, month, day), (hour, minute, second, 0), TimeZone::UTC)
}

/// `Mar  5 12:34` (a recent file: no year) or `Mar  5  2023` (an old one: no time).
fn classic(month: &str, rest: &mut &str, now: i64) -> Option<i64> {
    let month = month_number(month)?;
    let day = number(token(rest)?)?;
    let last = token(rest)?;
    if let Some((hour, minute)) = last.split_once(':') {
        let (hour, minute) = (number(hour)?, number(minute)?);
        let now_year = i64::from(
            Timestamp::from_second(now)
                .ok()?
                .to_zoned(TimeZone::UTC)
                .year(),
        );
        let this_year = at((now_year, month, day), (hour, minute, 0, 0), TimeZone::UTC)?;
        // `ls` shows the time for files from the last six months: a date in the future is last
        // year's (a day of slack for clocks that disagree).
        if this_year > now + 86_400 {
            at(
                (now_year - 1, month, day),
                (hour, minute, 0, 0),
                TimeZone::UTC,
            )
        } else {
            Some(this_year)
        }
    } else {
        at((number(last)?, month, day), (0, 0, 0, 0), TimeZone::UTC)
    }
}
