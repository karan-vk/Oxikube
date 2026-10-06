//! Reading typed values out of Table cell text.
//!
//! The server formats most cells as text: the pod table types `Ready`, `Restarts` and `Age` as
//! plain `string` (`1/1`, `3 (5m ago)`, `15h`). To sort `9` before `10` and `500Mi` before `2Gi`
//! the provider recovers the number from the text, once per cell read.

use jiff::Timestamp;
use oxikube_domain::{Age, Quantity};

use crate::columns::CellSort;

/// A column id from a server column name: lower case, runs of other characters become `-`.
/// `Created At` is `created-at`; a name with no letters or digits is `column`.
pub(super) fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "column".to_owned()
    } else {
        out
    }
}

/// A kubectl duration (`15h`, `3d5h`, `5m30s`, `2y10d`) as an [`Age`]; `None` for anything else
/// (`<unknown>`, `<none>`, free text).
pub(super) fn parse_age(text: &str) -> Option<Age> {
    let mut rest = text.trim();
    if rest.is_empty() {
        return None;
    }
    let mut secs: i64 = 0;
    while !rest.is_empty() {
        let digits = rest.find(|c: char| !c.is_ascii_digit())?;
        if digits == 0 {
            return None;
        }
        let n: i64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = match rest.chars().next()? {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 604_800,
            'y' => 31_536_000,
            _ => return None,
        };
        secs = secs.checked_add(n.checked_mul(unit)?)?;
        rest = &rest[1..];
    }
    Some(Age::from_secs(secs))
}

/// The sort key of an age-like cell: a kubectl duration, else an RFC 3339 timestamp, else text.
pub(super) fn age_sort(text: &str) -> CellSort {
    if let Some(age) = parse_age(text) {
        CellSort::Age(age)
    } else if let Ok(at) = text.parse::<Timestamp>() {
        CellSort::Time(at)
    } else if text.is_empty() {
        CellSort::None
    } else {
        CellSort::Text
    }
}

/// The sort key of a plain text cell, recovering the value the text spells:
///
/// * `42` is an integer, and so is the count before a note (`3 (5m ago)`);
/// * `1/2` is the ratio `0.5`;
/// * `500Mi`, `250m` and `1.5Gi` are quantities;
/// * anything else sorts as text, and an empty cell as blank.
pub(super) fn text_sort(text: &str) -> CellSort {
    let t = text.trim();
    if t.is_empty() {
        return CellSort::None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return CellSort::Int(n);
    }
    if let Some((count, note)) = t.split_once(" (")
        && note.ends_with(')')
        && let Ok(n) = count.parse::<i64>()
    {
        return CellSort::Int(n);
    }
    if let Some((a, b)) = t.split_once('/')
        && let (Ok(a), Ok(b)) = (a.parse::<u32>(), b.parse::<u32>())
    {
        let ratio = if b == 0 {
            0.0
        } else {
            f64::from(a) / f64::from(b)
        };
        return CellSort::Float(ratio);
    }
    if t.starts_with(|c: char| c.is_ascii_digit())
        && t.ends_with(|c: char| c.is_ascii_alphanumeric())
        && let Ok(q) = Quantity::parse(t)
    {
        return CellSort::Quantity(q);
    }
    CellSort::Text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_cells_recover_their_typed_sort_key() {
        assert_eq!(text_sort("42"), CellSort::Int(42));
        assert_eq!(text_sort("3 (5m ago)"), CellSort::Int(3));
        assert_eq!(text_sort("1/2"), CellSort::Float(0.5));
        assert_eq!(text_sort("0/0"), CellSort::Float(0.0));
        assert_eq!(text_sort(""), CellSort::None);
        assert_eq!(text_sort("Running"), CellSort::Text);
        assert!(matches!(text_sort("500Mi"), CellSort::Quantity(_)));
        assert!(matches!(text_sort("250m"), CellSort::Quantity(_)));
        assert_eq!(text_sort("10.244.0.6"), CellSort::Text);
        assert_eq!(text_sort("<none>"), CellSort::Text);
        assert_eq!(text_sort("web-1"), CellSort::Text);
    }

    #[test]
    fn kubectl_durations_parse_to_ages() {
        assert_eq!(parse_age("15h"), Some(Age::from_secs(15 * 3600)));
        assert_eq!(parse_age("5m30s"), Some(Age::from_secs(330)));
        assert_eq!(
            parse_age("3d5h"),
            Some(Age::from_secs(3 * 86_400 + 5 * 3600))
        );
        assert_eq!(
            parse_age("2y10d"),
            Some(Age::from_secs(2 * 31_536_000 + 10 * 86_400))
        );
        for bad in ["", "<unknown>", "h", "12", "5x", "abc"] {
            assert_eq!(parse_age(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn age_text_sorts_by_length_and_timestamps_by_time() {
        assert!(matches!(age_sort("5m"), CellSort::Age(_)));
        assert!(matches!(
            age_sort("2026-01-01T00:00:00Z"),
            CellSort::Time(_)
        ));
        assert_eq!(age_sort("<unknown>"), CellSort::Text);
        assert_eq!(age_sort(""), CellSort::None);
    }

    #[test]
    fn slugs_are_stable_kebab_case() {
        assert_eq!(slug("Created At"), "created-at");
        assert_eq!(slug("Name"), "name");
        assert_eq!(slug("  Pod/IP (v4)  "), "pod-ip-v4");
        assert_eq!(slug("%%%"), "column");
        assert_eq!(slug("Ready"), "ready");
    }
}
