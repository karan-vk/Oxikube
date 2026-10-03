// Portions of the test corpus (`kdash_corpus`) derive from kdash
// (https://github.com/kdash-rs/kdash, `src/app/utils.rs`), Copyright (c) 2021 Deepu K Sasidharan,
// MIT licence. The full notice is in THIRD_PARTY_NOTICES.md and next to the copied test data
// below. The kubectl cut-offs follow Kubernetes apimachinery `duration.HumanDuration`
// (Apache-2.0, semantics only). Modifications (c) Oxikube contributors.

//! Resource age: how long ago something was created, formatted the way tables show it.
//!
//! [`Age`] is a non-negative duration. [`Age::between`] takes both clocks as arguments, so
//! callers and tests never read the system time. Negative spans (clock skew between the cluster
//! and this machine) clamp to zero and render as `0s`.
//!
//! Two text styles exist because Kubernetes and kdash disagree:
//!
//! * [`AgeStyle::Kubectl`] (the [`Display`](std::fmt::Display) form) is `kubectl get`'s compact
//!   form: `45s`, `5m`, `3h`, `5d3h`, `2d`, `3y20d`.
//! * [`AgeStyle::Detailed`] keeps every unit up to a week (`1w3d1h`, `23h59m`), like kdash.
//!
//! Sort tables by [`Age`] itself (it is `Ord`) or [`Age::as_secs`], never by the text.
//!
//! # Performance
//!
//! Ages are recomputed for visible rows on a timer. Formatting writes into the caller's
//! formatter and allocates nothing beyond the output string.

use std::fmt;

use jiff::{SignedDuration, Timestamp};

const MINUTE: i64 = 60;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
const YEAR: i64 = 365 * DAY;

/// Which text form an [`Age`] renders in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AgeStyle {
    /// `kubectl get` style, from apimachinery `duration.HumanDuration`. The default.
    #[default]
    Kubectl,
    /// kdash style: weeks, days, hours and minutes, with minutes dropped once days appear.
    Detailed,
}

/// A non-negative span since a resource was created. See the [module docs](self).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Age(SignedDuration);

impl Age {
    /// A zero age.
    pub const ZERO: Age = Age(SignedDuration::ZERO);

    /// Age of something created at `created`, as seen at `now`. If `created` is in the future
    /// the age is zero.
    pub fn between(created: Timestamp, now: Timestamp) -> Self {
        Self::from_duration(now.duration_since(created))
    }

    /// Wraps a duration, clamping negative values to zero.
    pub fn from_duration(duration: SignedDuration) -> Self {
        if duration.is_negative() {
            Self::ZERO
        } else {
            Self(duration)
        }
    }

    /// Builds an age from whole seconds, clamping negative values to zero.
    pub fn from_secs(secs: i64) -> Self {
        Self::from_duration(SignedDuration::from_secs(secs))
    }

    /// The exact span, for sorting and arithmetic. Never negative.
    pub const fn as_duration(&self) -> SignedDuration {
        self.0
    }

    /// Whole seconds in the span (sub-second part dropped). Never negative.
    pub const fn as_secs(&self) -> i64 {
        self.0.as_secs()
    }

    /// Renders in the given style. `with_secs` only affects [`AgeStyle::Detailed`], which then
    /// appends seconds while the age is under an hour.
    pub fn format(&self, style: AgeStyle, with_secs: bool) -> String {
        let mut out = String::new();
        let _ = match style {
            AgeStyle::Kubectl => self.write_kubectl(&mut out),
            AgeStyle::Detailed => self.write_detailed(&mut out, with_secs),
        };
        out
    }

    /// `kubectl get` text, same as `to_string()`.
    pub fn to_kubectl_string(&self) -> String {
        self.format(AgeStyle::Kubectl, false)
    }

    /// kdash-style text; see [`AgeStyle::Detailed`]. Zero renders as `0s` with seconds, `0m`
    /// without.
    pub fn to_detailed_string(&self, with_secs: bool) -> String {
        self.format(AgeStyle::Detailed, with_secs)
    }

    /// Port of `duration.HumanDuration`: at most two units, switching unit at fixed cut-offs.
    fn write_kubectl(&self, out: &mut impl fmt::Write) -> fmt::Result {
        let secs = self.as_secs();
        let minutes = secs / MINUTE;
        let hours = secs / HOUR;
        let days = secs / DAY;
        if secs < 2 * MINUTE {
            write!(out, "{secs}s")
        } else if minutes < 10 {
            match secs % MINUTE {
                0 => write!(out, "{minutes}m"),
                s => write!(out, "{minutes}m{s}s"),
            }
        } else if minutes < 3 * 60 {
            write!(out, "{minutes}m")
        } else if hours < 8 {
            match minutes % 60 {
                0 => write!(out, "{hours}h"),
                m => write!(out, "{hours}h{m}m"),
            }
        } else if hours < 48 {
            write!(out, "{hours}h")
        } else if hours < 8 * 24 {
            match hours % 24 {
                0 => write!(out, "{days}d"),
                h => write!(out, "{days}d{h}h"),
            }
        } else if secs < 2 * YEAR {
            write!(out, "{days}d")
        } else if secs < 8 * YEAR {
            match days % 365 {
                0 => write!(out, "{}y", days / 365),
                d => write!(out, "{}y{d}d", days / 365),
            }
        } else {
            write!(out, "{}y", days / 365)
        }
    }

    /// Port of kdash `duration_to_age`.
    fn write_detailed(&self, out: &mut impl fmt::Write, with_secs: bool) -> fmt::Result {
        let secs = self.as_secs();
        let weeks = secs / WEEK;
        let days = (secs / DAY) % 7;
        let hours = (secs / HOUR) % 24;
        let mins = (secs / MINUTE) % 60;
        let rest_secs = secs % MINUTE;
        if weeks != 0 {
            write!(out, "{weeks}w")?;
        }
        if days != 0 {
            write!(out, "{days}d")?;
        }
        if hours != 0 {
            write!(out, "{hours}h")?;
        }
        let mut wrote = weeks != 0 || days != 0 || hours != 0;
        if mins != 0 && days == 0 && weeks == 0 {
            write!(out, "{mins}m")?;
            wrote = true;
        }
        if with_secs && rest_secs != 0 && hours == 0 && days == 0 && weeks == 0 {
            write!(out, "{rest_secs}s")?;
            wrote = true;
        }
        if wrote {
            Ok(())
        } else if with_secs {
            out.write_str("0s")
        } else {
            out.write_str("0m")
        }
    }
}

impl From<SignedDuration> for Age {
    fn from(duration: SignedDuration) -> Self {
        Self::from_duration(duration)
    }
}

/// `kubectl get` text. Negative spans were clamped to zero, so this never prints `<invalid>`.
impl fmt::Display for Age {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_kubectl(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn kubectl(secs: i64) -> String {
        Age::from_secs(secs).to_string()
    }

    #[test]
    fn kubectl_cut_offs() {
        let cases: &[(i64, &str)] = &[
            (0, "0s"),
            (1, "1s"),
            (45, "45s"),
            (119, "119s"),
            (120, "2m"),
            (121, "2m1s"),
            (5 * 60, "5m"),
            (9 * 60 + 59, "9m59s"),
            (10 * 60, "10m"),
            (10 * 60 + 30, "10m"),
            (179 * 60 + 59, "179m"),
            (3 * HOUR, "3h"),
            (3 * HOUR + 30 * MINUTE, "3h30m"),
            (7 * HOUR + 59 * MINUTE, "7h59m"),
            (8 * HOUR, "8h"),
            (8 * HOUR + 30 * MINUTE, "8h"),
            (47 * HOUR + 59 * MINUTE, "47h"),
            (48 * HOUR, "2d"),
            (5 * DAY + 3 * HOUR, "5d3h"),
            (5 * DAY + 3 * HOUR + 59 * MINUTE, "5d3h"),
            (7 * DAY + 23 * HOUR, "7d23h"),
            (8 * DAY, "8d"),
            (8 * DAY + 5 * HOUR, "8d"),
            (364 * DAY, "364d"),
            (2 * YEAR - 1, "729d"),
            (2 * YEAR, "2y"),
            (2 * YEAR + 20 * DAY, "2y20d"),
            (3 * YEAR + DAY, "3y1d"),
            (8 * YEAR - 1, "7y364d"),
            (8 * YEAR, "8y"),
            (8 * YEAR + 100 * DAY, "8y"),
            (100 * YEAR, "100y"),
        ];
        for (secs, want) in cases {
            assert_eq!(kubectl(*secs), *want, "{secs}s");
        }
    }

    #[test]
    fn negative_ages_render_zero() {
        assert_eq!(Age::from_secs(-1).to_string(), "0s");
        assert_eq!(Age::from_secs(i64::MIN).to_string(), "0s");
        let now = ts("2021-04-15T14:10:00Z");
        let future = ts("2021-04-15T14:10:30Z");
        assert_eq!(Age::between(future, now), Age::ZERO);
        assert_eq!(Age::between(future, now).to_string(), "0s");
        assert_eq!(
            Age::from_duration(SignedDuration::from_millis(-1500)),
            Age::ZERO
        );
        assert_eq!(Age::from_secs(-5).to_detailed_string(true), "0s");
        assert_eq!(Age::from_secs(-5).to_detailed_string(false), "0m");
    }

    #[test]
    fn between_takes_the_clock_as_parameter() {
        let created = ts("2021-04-12T11:10:10Z");
        let now = ts("2021-04-15T14:10:10Z");
        let age = Age::between(created, now);
        assert_eq!(age.as_secs(), 3 * DAY + 3 * HOUR);
        assert_eq!(age.to_string(), "3d3h");
        assert_eq!(
            age.as_duration(),
            SignedDuration::from_secs(3 * DAY + 3 * HOUR)
        );
        assert_eq!(Age::from(SignedDuration::from_secs(60)).to_string(), "60s");
    }

    #[test]
    fn sub_second_part_is_dropped() {
        let age = Age::from_duration(SignedDuration::from_millis(119_999));
        assert_eq!(age.to_string(), "119s");
        assert!(age > Age::from_secs(119));
    }

    #[test]
    fn sorts_by_duration_not_text() {
        let mut ages = [
            Age::from_secs(9 * DAY),
            Age::from_secs(45),
            Age::from_secs(2 * HOUR),
            Age::from_secs(100 * DAY),
            Age::from_secs(10 * MINUTE),
        ];
        ages.sort();
        let secs: Vec<i64> = ages.iter().map(Age::as_secs).collect();
        assert_eq!(secs, vec![45, 600, 7200, 9 * DAY, 100 * DAY]);
        let mut texts: Vec<String> = ages.iter().map(ToString::to_string).collect();
        let by_age = texts.clone();
        texts.sort();
        assert_ne!(texts, by_age, "text order differs from age order");
    }

    #[test]
    fn style_default_and_helpers() {
        assert_eq!(AgeStyle::default(), AgeStyle::Kubectl);
        let age = Age::from_secs(5 * DAY + 3 * HOUR);
        assert_eq!(age.to_kubectl_string(), "5d3h");
        assert_eq!(age.format(AgeStyle::Kubectl, true), "5d3h");
        assert_eq!(age.to_detailed_string(false), "5d3h");
    }

    /// Test data below is derived from kdash `src/app/utils.rs` (`test_to_age`,
    /// `test_to_age_secs`), MIT licence.
    ///
    /// Copyright (c) 2021 Deepu K Sasidharan
    ///
    /// Permission is hereby granted, free of charge, to any person obtaining a copy of this
    /// software and associated documentation files (the "Software"), to deal in the Software
    /// without restriction, including without limitation the rights to use, copy, modify, merge,
    /// publish, distribute, sublicense, and/or sell copies of the Software, and to permit
    /// persons to whom the Software is furnished to do so, subject to the following conditions:
    /// The above copyright notice and this permission notice shall be included in all copies or
    /// substantial portions of the Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY
    /// OF ANY KIND, EXPRESS OR IMPLIED.
    mod kdash_corpus {
        use super::*;

        /// `(created, now, to_age_secs, to_age)` from kdash, with dates converted to RFC 3339.
        const CASES: &[(&str, &str, &str, &str)] = &[
            ("2021-04-15T14:09:10Z", "2021-04-15T14:10:00Z", "50s", "0m"),
            (
                "2021-04-15T14:08:10Z",
                "2021-04-15T14:10:00Z",
                "1m50s",
                "1m",
            ),
            ("2021-04-15T14:09:00Z", "2021-04-15T14:10:00Z", "1m", "1m"),
            ("2021-04-15T13:50:00Z", "2021-04-15T14:10:00Z", "20m", "20m"),
            (
                "2021-04-15T13:50:10Z",
                "2021-04-15T14:10:00Z",
                "19m50s",
                "19m",
            ),
            (
                "2021-04-15T10:50:10Z",
                "2021-04-15T14:10:00Z",
                "3h19m",
                "3h19m",
            ),
            ("2021-04-14T15:10:10Z", "2021-04-15T14:10:10Z", "23h", "23h"),
            (
                "2021-04-14T14:11:10Z",
                "2021-04-15T14:10:10Z",
                "23h59m",
                "23h59m",
            ),
            ("2021-04-14T14:10:10Z", "2021-04-15T14:10:10Z", "1d", "1d"),
            ("2021-04-12T14:10:10Z", "2021-04-15T14:10:10Z", "3d", "3d"),
            ("2021-04-12T13:50:10Z", "2021-04-15T14:10:10Z", "3d", "3d"),
            (
                "2021-04-12T11:10:10Z",
                "2021-04-15T14:10:10Z",
                "3d3h",
                "3d3h",
            ),
            (
                "2021-04-12T10:50:10Z",
                "2021-04-15T14:10:00Z",
                "3d3h",
                "3d3h",
            ),
            ("2021-04-08T14:10:10Z", "2021-04-15T14:10:10Z", "1w", "1w"),
            (
                "2021-04-05T12:30:10Z",
                "2021-04-15T14:10:10Z",
                "1w3d1h",
                "1w3d1h",
            ),
            (
                "1970-01-01T00:00:00Z",
                "2021-04-15T14:10:00Z",
                "2676w14h",
                "2676w14h",
            ),
        ];

        #[test]
        fn detailed_style_matches_kdash() {
            for (created, now, with_secs, without_secs) in CASES {
                let age = Age::between(ts(created), ts(now));
                assert_eq!(
                    age.to_detailed_string(true),
                    *with_secs,
                    "{created} -> {now}"
                );
                assert_eq!(
                    age.to_detailed_string(false),
                    *without_secs,
                    "{created} -> {now}"
                );
            }
        }

        #[test]
        fn zero_age_matches_kdash() {
            assert_eq!(Age::ZERO.to_detailed_string(true), "0s");
            assert_eq!(Age::ZERO.to_detailed_string(false), "0m");
        }

        /// The same corpus inputs rendered with the kubectl cut-offs (a different, coarser
        /// style than kdash's, so the expected text differs where the rules do).
        #[test]
        fn kubectl_style_on_corpus_inputs() {
            let cases: &[(&str, &str, &str)] = &[
                ("2021-04-15T14:09:10Z", "2021-04-15T14:10:00Z", "50s"),
                ("2021-04-15T14:08:10Z", "2021-04-15T14:10:00Z", "110s"),
                ("2021-04-15T13:50:00Z", "2021-04-15T14:10:00Z", "20m"),
                ("2021-04-15T10:50:10Z", "2021-04-15T14:10:00Z", "3h19m"),
                ("2021-04-14T15:10:10Z", "2021-04-15T14:10:10Z", "23h"),
                ("2021-04-14T14:11:10Z", "2021-04-15T14:10:10Z", "23h"),
                ("2021-04-14T14:10:10Z", "2021-04-15T14:10:10Z", "24h"),
                ("2021-04-12T14:10:10Z", "2021-04-15T14:10:10Z", "3d"),
                ("2021-04-12T11:10:10Z", "2021-04-15T14:10:10Z", "3d3h"),
                ("2021-04-08T14:10:10Z", "2021-04-15T14:10:10Z", "7d"),
                ("2021-04-05T12:30:10Z", "2021-04-15T14:10:10Z", "10d"),
                ("1970-01-01T00:00:00Z", "2021-04-15T14:10:00Z", "51y"),
            ];
            for (created, now, want) in cases {
                assert_eq!(
                    Age::between(ts(created), ts(now)).to_string(),
                    *want,
                    "{created} -> {now}"
                );
            }
        }
    }

    /// Rank of the leading unit of a kubectl-style string.
    fn leading_unit_rank(text: &str) -> u8 {
        let unit = text
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .chars()
            .next();
        match unit {
            Some('s') => 0,
            Some('m') => 1,
            Some('h') => 2,
            Some('d') => 3,
            Some('y') => 4,
            other => panic!("unexpected unit {other:?} in {text:?}"),
        }
    }

    #[test]
    fn leading_unit_changes_exactly_at_the_documented_boundaries() {
        for (secs, rank_before, rank_after) in [
            (2 * MINUTE, 0, 1),
            (3 * HOUR, 1, 2),
            (48 * HOUR, 2, 3),
            (2 * YEAR, 3, 4),
        ] {
            assert_eq!(leading_unit_rank(&kubectl(secs - 1)), rank_before, "{secs}");
            assert_eq!(leading_unit_rank(&kubectl(secs)), rank_after, "{secs}");
        }
    }

    proptest! {
        #[test]
        fn leading_unit_never_decreases(a in 0i64..(200 * YEAR), b in 0i64..(200 * YEAR)) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(leading_unit_rank(&kubectl(lo)) <= leading_unit_rank(&kubectl(hi)));
        }

        #[test]
        fn ord_matches_seconds(a in any::<i64>(), b in any::<i64>()) {
            let (x, y) = (Age::from_secs(a), Age::from_secs(b));
            prop_assert_eq!(x.cmp(&y), a.max(0).cmp(&b.max(0)));
        }

        #[test]
        fn formats_never_panic_and_are_not_empty(secs in any::<i64>(), with_secs in any::<bool>()) {
            let age = Age::from_secs(secs);
            prop_assert!(!age.to_string().is_empty());
            prop_assert!(!age.to_detailed_string(with_secs).is_empty());
        }

        #[test]
        fn kubectl_text_is_within_one_unit_of_the_age(secs in 0i64..(100 * YEAR)) {
            // Parse the leading number and unit back and check the approximation error is
            // smaller than the leading unit.
            let text = kubectl(secs);
            let digits: String = text.chars().take_while(char::is_ascii_digit).collect();
            let n: i64 = digits.parse().unwrap();
            let (unit, in_unit) = match leading_unit_rank(&text) {
                0 => (1, 1),
                1 => (MINUTE, MINUTE),
                2 => (HOUR, HOUR),
                3 => (DAY, DAY),
                _ => (YEAR, YEAR),
            };
            let approx = n * unit;
            prop_assert!(approx <= secs && secs - approx < in_unit.max(1) * 2,
                "{} rendered as {} (approx {})", secs, text, approx);
        }
    }
}
