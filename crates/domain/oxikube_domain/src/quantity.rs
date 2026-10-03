// Portions derived from kubectl-view-allocations `qty::Qty`
// (https://github.com/davidB/kubectl-view-allocations), released under CC0-1.0.
// The scale table and the "value plus scale" shape follow that parser; the stored value is an
// exact integer here (no `f64`), and exponent forms, signs and canonical formatting follow
// Kubernetes apimachinery `resource.Quantity` (Apache-2.0, semantics only).
// Modifications (c) Oxikube contributors.

//! Exact Kubernetes resource quantities (`500m`, `1.5Gi`, `129e6`).
//!
//! A [`Quantity`] stores its value as a whole number of **nano-units** in an `i128`, so
//! `0.1 + 0.2` style errors cannot appear and every value the Kubernetes API can express with
//! up to nano precision (`1n` to well beyond `9Ei`) is represented exactly. Values with more than
//! nine fractional digits are rounded **away from zero** to the next nano, matching apimachinery
//! (`1.5n` parses as `2n`).
//!
//! Floating point appears only in display helpers ([`Quantity::as_f64`],
//! [`Quantity::percent_of`], [`Quantity::human_bytes`]); never compare or sum those results.
//!
//! Equality, ordering and hashing look at the numeric value only: `1Gi == 1024Mi` and
//! `1000m == 1`. The [`QuantityFormat`] a quantity was parsed with only steers the canonical
//! text produced by [`Display`](std::fmt::Display).
//!
//! # Performance
//!
//! Parsing never allocates (it scans the input once; errors allocate). Canonical formatting
//! writes straight into the formatter, so the only allocation is the output string the caller
//! asks for. Quantities are `Copy`, so metrics samples can be recomputed per tick cheaply.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;

/// Nano-units per whole unit.
const NANO: i128 = 1_000_000_000;

/// Largest power of ten that fits in a `u128` (`10^38`).
const MAX_POW10: u32 = 38;

/// Binary suffixes, ascending: `Ki` is `2^10`, `Ei` is `2^60`.
const BINARY_SUFFIXES: [&str; 6] = ["Ki", "Mi", "Gi", "Ti", "Pi", "Ei"];

/// Decimal suffixes as `(suffix, power of ten)`, descending, the canonical `DecimalSI` set.
const DECIMAL_SUFFIXES: [(&str, i32); 10] = [
    ("E", 18),
    ("P", 15),
    ("T", 12),
    ("G", 9),
    ("M", 6),
    ("k", 3),
    ("", 0),
    ("m", -3),
    ("u", -6),
    ("n", -9),
];

/// Exponents beyond this magnitude are rejected without parsing further digits.
const EXPONENT_CAP: i64 = 10_000;

/// Why a string is not a valid [`Quantity`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuantityError {
    /// The input was empty.
    #[error("quantity is empty")]
    Empty,
    /// The numeric part is missing or malformed (`""`, `"-"`, `"--1"`, `"."`).
    #[error("invalid number in quantity {0:?}")]
    InvalidNumber(String),
    /// The suffix is not a Kubernetes quantity suffix or exponent (`"Gb"`, `".3"`).
    #[error("invalid suffix {0:?} in quantity")]
    InvalidSuffix(String),
    /// The value does not fit in the nano-unit `i128` representation.
    #[error("quantity {0:?} is out of range")]
    OutOfRange(String),
}

/// The notation family a [`Quantity`] prints in, mirroring apimachinery `resource.Format`.
///
/// It does not affect equality, ordering or arithmetic values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum QuantityFormat {
    /// Powers of 1024: `Ki Mi Gi Ti Pi Ei`.
    BinarySI,
    /// Powers of 1000: `n u m "" k M G T P E`. The default for plain numbers.
    #[default]
    DecimalSI,
    /// Scientific form with an exponent that is a multiple of three: `12e6`.
    DecimalExponent,
}

/// An exact Kubernetes resource quantity. See the [module docs](self).
#[derive(Debug, Clone, Copy, Default)]
pub struct Quantity {
    /// Value in nano-units.
    nanos: i128,
    /// Notation used by `Display`.
    format: QuantityFormat,
}

impl Quantity {
    /// The zero quantity in `DecimalSI`.
    pub const ZERO: Quantity = Quantity {
        nanos: 0,
        format: QuantityFormat::DecimalSI,
    };

    /// Builds a quantity from a raw count of nano-units.
    pub const fn from_nanos(nanos: i128) -> Self {
        Self {
            nanos,
            format: QuantityFormat::DecimalSI,
        }
    }

    /// Builds a quantity from milli-units (`1500` is `1500m`, one and a half cores).
    pub const fn from_milli(milli: i64) -> Self {
        Self::from_nanos(milli as i128 * 1_000_000)
    }

    /// Builds a quantity from whole units (bytes, cores, pods).
    pub const fn from_value(value: i64) -> Self {
        Self::from_nanos(value as i128 * NANO)
    }

    /// Returns the same value with a different canonical notation.
    #[must_use]
    pub const fn with_format(mut self, format: QuantityFormat) -> Self {
        self.format = format;
        self
    }

    /// The notation family this quantity prints in.
    pub const fn format(&self) -> QuantityFormat {
        self.format
    }

    /// Parses a Kubernetes quantity string.
    ///
    /// Accepts an optional sign, digits with an optional fraction (`1.5`, `.5`, `1.`), and one
    /// of: no suffix, a decimal suffix (`n u m k M G T P E`), a binary suffix
    /// (`Ki Mi Gi Ti Pi Ei`) or an exponent (`1e3`, `12E6`, `5e-3`). Whitespace is not allowed.
    ///
    /// # Errors
    ///
    /// Returns a [`QuantityError`] for empty input, a bad number, an unknown suffix, or a value
    /// beyond the `i128` nano-unit range.
    pub fn parse(input: &str) -> Result<Self, QuantityError> {
        if input.is_empty() {
            return Err(QuantityError::Empty);
        }
        let bytes = input.as_bytes();
        let mut i = 0;
        let mut negative = false;
        match bytes[0] {
            b'+' => i = 1,
            b'-' => {
                negative = true;
                i = 1;
            }
            _ => {}
        }
        let int_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let int_part = &input[int_start..i];
        let mut frac_part = "";
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            let frac_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            frac_part = &input[frac_start..i];
        }
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(QuantityError::InvalidNumber(input.to_owned()));
        }
        let (format, shift, dec_exp) = parse_suffix(&input[i..])
            .ok_or_else(|| QuantityError::InvalidSuffix(input[i..].to_owned()))?;

        // Trailing fractional zeros carry no information; drop them so long inputs such as
        // `1.000000000000000000000000000000000000000000` stay in range.
        let frac_part = frac_part.trim_end_matches('0');
        let out_of_range = || QuantityError::OutOfRange(input.to_owned());
        let mut digits: u128 = 0;
        for b in int_part.bytes().chain(frac_part.bytes()) {
            digits = digits
                .checked_mul(10)
                .and_then(|d| d.checked_add(u128::from(b - b'0')))
                .ok_or_else(out_of_range)?;
        }
        if digits == 0 {
            return Ok(Self { nanos: 0, format });
        }
        // value (in nano-units) = digits * 2^shift * 10^(dec_exp + 9 - frac_len)
        let e10 = i64::from(dec_exp) + 9 - frac_part.len() as i64;
        let scaled = digits
            .checked_mul(1u128 << shift)
            .ok_or_else(out_of_range)?;
        let magnitude = if e10 >= 0 {
            let p = u32::try_from(e10).ok().filter(|p| *p <= MAX_POW10);
            let p = p.ok_or_else(out_of_range)?;
            scaled.checked_mul(10u128.pow(p)).ok_or_else(out_of_range)?
        } else {
            // Round away from zero: anything below one nano becomes one nano.
            match u32::try_from(-e10).ok().filter(|p| *p <= MAX_POW10) {
                Some(p) => {
                    let unit = 10u128.pow(p);
                    scaled / unit + u128::from(scaled % unit != 0)
                }
                None => 1,
            }
        };
        let nanos = if negative {
            0i128.checked_sub_unsigned(magnitude)
        } else {
            i128::try_from(magnitude).ok()
        };
        Ok(Self {
            nanos: nanos.ok_or_else(out_of_range)?,
            format,
        })
    }

    /// The exact value in nano-units.
    pub const fn nanos(&self) -> i128 {
        self.nanos
    }

    /// The value in whole units, rounded up like apimachinery `Value()` (`1500m` is `2`).
    pub fn value(&self) -> i128 {
        div_ceil_i128(self.nanos, NANO)
    }

    /// The value in milli-units, rounded up like apimachinery `MilliValue()`
    /// (`1500u` is `2`, `1Gi` is `1073741824000`).
    pub fn milli_value(&self) -> i128 {
        div_ceil_i128(self.nanos, 1_000_000)
    }

    /// Whether the value is exactly zero.
    pub const fn is_zero(&self) -> bool {
        self.nanos == 0
    }

    /// Whether the value is below zero.
    pub const fn is_negative(&self) -> bool {
        self.nanos < 0
    }

    /// The value as a float. **For display only**: it loses precision beyond 2^53 nano-units;
    /// never use it for comparison or arithmetic.
    pub fn as_f64(&self) -> f64 {
        self.nanos as f64 / NANO as f64
    }

    /// Utilisation of `self` against `total` in percent (`250m` of `1` is `25.0`), or `None`
    /// when `total` is zero. The result is a float for display only.
    pub fn percent_of(&self, total: &Quantity) -> Option<f64> {
        if total.nanos == 0 {
            return None;
        }
        Some(self.nanos as f64 * 100.0 / total.nanos as f64)
    }

    /// Sum, or `None` on overflow. The result keeps `self`'s notation (the other operand's when
    /// `self` is zero).
    pub fn checked_add(&self, other: &Quantity) -> Option<Quantity> {
        Some(Quantity {
            nanos: self.nanos.checked_add(other.nanos)?,
            format: self.format_for_sum(other),
        })
    }

    /// Difference, or `None` on overflow. Notation follows [`Quantity::checked_add`].
    pub fn checked_sub(&self, other: &Quantity) -> Option<Quantity> {
        Some(Quantity {
            nanos: self.nanos.checked_sub(other.nanos)?,
            format: self.format_for_sum(other),
        })
    }

    fn format_for_sum(&self, other: &Quantity) -> QuantityFormat {
        if self.nanos == 0 {
            other.format
        } else {
            self.format
        }
    }

    /// Formats a byte count for a table cell: `512 B`, `1.5 GiB`, `100 MiB`.
    ///
    /// Uses binary units with at most one decimal. This is UI text, not the Kubernetes form;
    /// use [`Display`](fmt::Display) for the API form.
    pub fn human_bytes(&self) -> String {
        const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
        let mut v = self.as_f64().abs();
        let mut unit = 0;
        while unit + 1 < UNITS.len() && v >= 1024.0 {
            v /= 1024.0;
            unit += 1;
        }
        // Round to one decimal, then carry if rounding reached the next unit (1023.97 KiB).
        let mut rounded = (v * 10.0).round() / 10.0;
        if rounded >= 1024.0 && unit + 1 < UNITS.len() {
            rounded /= 1024.0;
            unit += 1;
        }
        let sign = if self.nanos < 0 && rounded > 0.0 {
            "-"
        } else {
            ""
        };
        if rounded.fract() == 0.0 {
            format!("{sign}{rounded:.0} {}", UNITS[unit])
        } else {
            format!("{sign}{rounded:.1} {}", UNITS[unit])
        }
    }

    /// Formats a CPU quantity for a table cell the way `kubectl top` does: whole millicores
    /// (`250m`, `1500m`, `0`). Sub-millicore values round up.
    pub fn human_cpu(&self) -> String {
        match self.milli_value() {
            0 => "0".to_owned(),
            m => format!("{m}m"),
        }
    }

    /// Writes the canonical Kubernetes text.
    fn write_canonical(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.nanos == 0 {
            return f.write_str("0");
        }
        if self.format == QuantityFormat::BinarySI && self.nanos % NANO == 0 {
            let v = self.nanos / NANO;
            for (k, suffix) in BINARY_SUFFIXES.iter().enumerate().rev() {
                let unit = 1i128 << (10 * (k + 1));
                if v % unit == 0 {
                    return write!(f, "{}{suffix}", v / unit);
                }
            }
            return write!(f, "{v}");
        }
        if self.format == QuantityFormat::DecimalExponent {
            for step in (-3..=9i32).rev() {
                let exp = step * 3;
                let unit = 10i128.pow((exp + 9) as u32);
                if self.nanos % unit == 0 {
                    let m = self.nanos / unit;
                    return if exp == 0 {
                        write!(f, "{m}")
                    } else {
                        write!(f, "{m}e{exp}")
                    };
                }
            }
        }
        // DecimalSI, and BinarySI values that are not whole numbers.
        for (suffix, exp) in DECIMAL_SUFFIXES {
            let unit = 10i128.pow((exp + 9) as u32);
            if self.nanos % unit == 0 {
                return write!(f, "{}{suffix}", self.nanos / unit);
            }
        }
        unreachable!("the `n` suffix divides every nano count")
    }
}

/// Maps a suffix to `(format, binary shift, decimal exponent)`.
fn parse_suffix(suffix: &str) -> Option<(QuantityFormat, u32, i32)> {
    if let Some(k) = BINARY_SUFFIXES.iter().position(|s| *s == suffix) {
        return Some((QuantityFormat::BinarySI, 10 * (k as u32 + 1), 0));
    }
    if let Some((_, exp)) = DECIMAL_SUFFIXES.iter().find(|(s, _)| *s == suffix) {
        return Some((QuantityFormat::DecimalSI, 0, *exp));
    }
    // `E` alone is exa (handled above); `E`/`e` followed by a signed integer is an exponent.
    let rest = suffix.strip_prefix(['e', 'E'])?;
    let (negative, digits) = match rest.as_bytes().first()? {
        b'+' => (false, &rest[1..]),
        b'-' => (true, &rest[1..]),
        _ => (false, rest),
    };
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut exp: i64 = 0;
    for b in digits.bytes() {
        exp = (exp * 10 + i64::from(b - b'0')).min(EXPONENT_CAP);
    }
    let exp = if negative { -exp } else { exp };
    Some((QuantityFormat::DecimalExponent, 0, exp as i32))
}

/// Division rounding toward positive infinity (`i128::div_ceil` is not stable).
fn div_ceil_i128(a: i128, b: i128) -> i128 {
    let q = a / b;
    if a % b != 0 && (a > 0) == (b > 0) {
        q + 1
    } else {
        q
    }
}

impl FromStr for Quantity {
    type Err = QuantityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// Canonical Kubernetes form: the largest suffix of the quantity's [`QuantityFormat`] that
/// keeps the mantissa an integer (`1536Mi`, `1500m`, `1k`, `12e6`). `parse(q.to_string()) == q`.
impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_canonical(f)
    }
}

impl PartialEq for Quantity {
    fn eq(&self, other: &Self) -> bool {
        self.nanos == other.nanos
    }
}

impl Eq for Quantity {}

impl Hash for Quantity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.nanos.hash(state);
    }
}

impl PartialOrd for Quantity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Quantity {
    fn cmp(&self, other: &Self) -> Ordering {
        self.nanos.cmp(&other.nanos)
    }
}

/// Saturating addition; use [`Quantity::checked_add`] to detect overflow.
impl Add for Quantity {
    type Output = Quantity;

    fn add(self, rhs: Quantity) -> Quantity {
        Quantity {
            nanos: self.nanos.saturating_add(rhs.nanos),
            format: self.format_for_sum(&rhs),
        }
    }
}

/// Saturating subtraction; use [`Quantity::checked_sub`] to detect overflow.
impl Sub for Quantity {
    type Output = Quantity;

    fn sub(self, rhs: Quantity) -> Quantity {
        Quantity {
            nanos: self.nanos.saturating_sub(rhs.nanos),
            format: self.format_for_sum(&rhs),
        }
    }
}

impl AddAssign for Quantity {
    fn add_assign(&mut self, rhs: Quantity) {
        *self = *self + rhs;
    }
}

impl SubAssign for Quantity {
    fn sub_assign(&mut self, rhs: Quantity) {
        *self = *self - rhs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn q(s: &str) -> Quantity {
        Quantity::parse(s).unwrap_or_else(|e| panic!("{s:?} should parse: {e}"))
    }

    #[test]
    fn parses_every_suffix() {
        let cases: &[(&str, i128)] = &[
            ("1n", 1),
            ("1u", 1_000),
            ("1m", 1_000_000),
            ("1", NANO),
            ("1k", NANO * 1_000),
            ("1M", NANO * 1_000_000),
            ("1G", NANO * 1_000_000_000),
            ("1T", NANO * 1_000_000_000_000),
            ("1P", NANO * 1_000_000_000_000_000),
            ("1E", NANO * 1_000_000_000_000_000_000),
            ("1Ki", NANO << 10),
            ("1Mi", NANO << 20),
            ("1Gi", NANO << 30),
            ("1Ti", NANO << 40),
            ("1Pi", NANO << 50),
            ("1Ei", NANO << 60),
        ];
        for (s, nanos) in cases {
            assert_eq!(q(s).nanos(), *nanos, "{s}");
        }
    }

    #[test]
    fn parses_exponents_decimals_and_signs() {
        let cases: &[(&str, i128)] = &[
            ("1e3", 1_000 * NANO),
            ("12E6", 12_000_000 * NANO),
            ("129e6", 129_000_000 * NANO),
            ("5e-3", 5_000_000),
            ("1e+2", 100 * NANO),
            ("1.5Gi", 3 * (NANO << 29)),
            ("0.5", NANO / 2),
            (".5", NANO / 2),
            ("1.", NANO),
            ("0.1", 100_000_000),
            ("1.5e3", 1_500 * NANO),
            ("+3", 3 * NANO),
            ("-3", -3 * NANO),
            ("-1.5Gi", -3 * (NANO << 29)),
            ("0", 0),
            ("-0", 0),
            ("0Gi", 0),
            ("0.000", 0),
            ("100m", 100_000_000),
            ("1.000000000000000000000000000000000000000000", NANO),
        ];
        for (s, nanos) in cases {
            assert_eq!(q(s).nanos(), *nanos, "{s}");
        }
    }

    #[test]
    fn handles_extremes() {
        assert_eq!(q("9Ei").nanos(), 9 * (NANO << 60));
        assert_eq!(q("1n").nanos(), 1);
        // Sub-nano fractions round away from zero.
        assert_eq!(q("1.5n").nanos(), 2);
        assert_eq!(q("0.0000000001").nanos(), 1);
        assert_eq!(q("-0.0000000001").nanos(), -1);
        assert_eq!(q("1e-99999999999").nanos(), 1);
        assert_eq!(q("0e99999999999").nanos(), 0);
        assert!(matches!(
            Quantity::parse("1e99999999999"),
            Err(QuantityError::OutOfRange(_))
        ));
        assert!(matches!(
            Quantity::parse("1e30"),
            Err(QuantityError::OutOfRange(_))
        ));
        assert!(matches!(
            Quantity::parse(&"9".repeat(60)),
            Err(QuantityError::OutOfRange(_))
        ));
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(Quantity::parse(""), Err(QuantityError::Empty));
        for s in [
            "1Gb", "1.2.3", "--1", "-", "+", ".", "-.", "Gi", "G", "e3", "1 Gi", " 1", "1 ", "1ki",
            "1gi", "1K", "1g", "1e", "1ee3", "1e3.5", "1e+", "1e-", "1Mib", "0x10", "NaN", "inf",
            "1,5", "1_000", "１",
        ] {
            assert!(Quantity::parse(s).is_err(), "{s:?} should be rejected");
        }
        assert!(matches!(
            Quantity::parse("1Gb"),
            Err(QuantityError::InvalidSuffix(s)) if s == "Gb"
        ));
        assert!(matches!(
            Quantity::parse("--1"),
            Err(QuantityError::InvalidNumber(_))
        ));
    }

    #[test]
    fn mixed_suffixes_compare_by_value() {
        assert_eq!(q("1Gi"), q("1024Mi"));
        assert_eq!(q("1000m"), q("1"));
        assert_eq!(q("1k"), q("1000"));
        assert_eq!(q("1e3"), q("1k"));
        assert!(q("1G") < q("1Gi"));
        assert!(q("999m") < q("1"));
        assert!(q("-1") < q("0"));
        assert_eq!(q("0Ki"), q("0m"));
    }

    #[test]
    fn canonical_formatting() {
        let cases: &[(&str, &str)] = &[
            ("0", "0"),
            ("0Gi", "0"),
            ("1Gi", "1Gi"),
            ("1024Mi", "1Gi"),
            ("1.5Gi", "1536Mi"),
            ("1000Mi", "1000Mi"),
            ("1000", "1k"),
            ("1500", "1500"),
            ("1500m", "1500m"),
            ("1000m", "1"),
            ("0.5", "500m"),
            ("1500u", "1500u"),
            ("1n", "1n"),
            ("129e6", "129e6"),
            ("12E6", "12e6"),
            ("1e3", "1e3"),
            ("1e-3", "1e-3"),
            ("1500e-3", "1500e-3"),
            ("0.001e3", "1"),
            ("-1Gi", "-1Gi"),
            ("-250m", "-250m"),
            ("9Ei", "9Ei"),
            ("100", "100"),
            ("3Ki", "3Ki"),
            ("2000k", "2M"),
            ("1E", "1E"),
        ];
        for (input, want) in cases {
            assert_eq!(q(input).to_string(), *want, "{input}");
        }
        // A binary quantity that is not a whole number falls back to decimal SI.
        let frac = Quantity::from_milli(500).with_format(QuantityFormat::BinarySI);
        assert_eq!(frac.to_string(), "500m");
    }

    #[test]
    fn canonical_round_trips_for_table_cases() {
        for s in [
            "0",
            "1",
            "-1",
            "500m",
            "1.5Gi",
            "129e6",
            "9Ei",
            "1n",
            "7Ki",
            "123456789u",
            "1E",
            "1e-9",
            "2.5e3",
            "100Mi",
        ] {
            let original = q(s);
            assert_eq!(q(&original.to_string()), original, "{s}");
        }
    }

    #[test]
    fn value_helpers_round_up() {
        assert_eq!(q("1500m").value(), 2);
        assert_eq!(q("-1500m").value(), -1);
        assert_eq!(q("1500u").milli_value(), 2);
        assert_eq!(q("1Gi").milli_value(), 1_073_741_824_000);
        assert_eq!(q("9Ei").value(), 9 << 60);
        assert_eq!(q("0").value(), 0);
        assert_eq!(q("1n").value(), 1);
        assert_eq!(q("250m").milli_value(), 250);
        assert!((q("0.1").as_f64() - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn arithmetic_is_exact() {
        // 0.1 + 0.2 == 0.3 exactly, unlike f64.
        assert_eq!(q("0.1") + q("0.2"), q("0.3"));
        assert_eq!(q("100m") + q("200m"), q("300m"));
        assert_eq!(q("1Gi") - q("512Mi"), q("512Mi"));
        assert_eq!(
            q("1Gi") + q("1G"),
            Quantity::from_nanos((NANO << 30) + NANO * 1_000_000_000)
        );
        assert_eq!(q("1") - q("1000m"), Quantity::ZERO);
        assert!((q("1") - q("2")).is_negative());
        let mut acc = Quantity::ZERO;
        acc += q("250m");
        acc += q("250m");
        acc -= q("100m");
        assert_eq!(acc, q("400m"));
    }

    #[test]
    fn arithmetic_notation_and_overflow() {
        assert_eq!((q("1Gi") + q("1")).format(), QuantityFormat::BinarySI);
        assert_eq!(
            (Quantity::ZERO + q("1Gi")).format(),
            QuantityFormat::BinarySI
        );
        let max = Quantity::from_nanos(i128::MAX);
        assert_eq!(max.checked_add(&q("1n")), None);
        assert_eq!(max + q("1n"), max);
        let min = Quantity::from_nanos(i128::MIN);
        assert_eq!(min.checked_sub(&q("1n")), None);
        assert_eq!(q("1").checked_add(&q("1")), Some(q("2")));
        assert_eq!(q("3").checked_sub(&q("1")), Some(q("2")));
    }

    #[test]
    fn percent_of() {
        assert_eq!(q("250m").percent_of(&q("1")), Some(25.0));
        assert_eq!(q("512Mi").percent_of(&q("1Gi")), Some(50.0));
        assert_eq!(q("1").percent_of(&Quantity::ZERO), None);
        assert_eq!(Quantity::ZERO.percent_of(&Quantity::ZERO), None);
        assert_eq!(Quantity::ZERO.percent_of(&q("1")), Some(0.0));
        assert_eq!(q("2").percent_of(&q("1")), Some(200.0));
    }

    #[test]
    fn table_cell_formatters() {
        assert_eq!(q("0").human_bytes(), "0 B");
        assert_eq!(q("512").human_bytes(), "512 B");
        assert_eq!(q("1Ki").human_bytes(), "1 KiB");
        assert_eq!(q("1.5Gi").human_bytes(), "1.5 GiB");
        assert_eq!(q("100Mi").human_bytes(), "100 MiB");
        assert_eq!(q("1023.99Ki").human_bytes(), "1 MiB");
        assert_eq!(q("9Ei").human_bytes(), "9 EiB");
        assert_eq!(q("-1Mi").human_bytes(), "-1 MiB");
        assert_eq!(q("250m").human_cpu(), "250m");
        assert_eq!(q("1.5").human_cpu(), "1500m");
        assert_eq!(q("126632173n").human_cpu(), "127m");
        assert_eq!(q("0").human_cpu(), "0");
        assert_eq!(q("8").human_cpu(), "8000m");
    }

    #[test]
    fn sorts_numerically_not_lexically() {
        let mut v = vec![q("10Mi"), q("9Mi"), q("1Gi"), q("500Ki"), q("100")];
        v.sort();
        let want = vec![q("100"), q("500Ki"), q("9Mi"), q("10Mi"), q("1Gi")];
        assert_eq!(v, want);
        let mut strings: Vec<String> = v.iter().map(ToString::to_string).collect();
        strings.sort();
        assert_ne!(
            strings,
            want.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "string order differs from numeric order"
        );
        let mut cpu = vec![q("1"), q("900m"), q("2500m"), q("100m")];
        cpu.sort();
        assert_eq!(cpu, vec![q("100m"), q("900m"), q("1"), q("2500m")]);
    }

    #[test]
    fn hash_follows_equality() {
        use std::collections::HashSet;
        let set: HashSet<Quantity> = [q("1Gi"), q("1024Mi"), q("1073741824")]
            .into_iter()
            .collect();
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn from_str_and_default() {
        assert_eq!("2Gi".parse::<Quantity>(), Ok(q("2Gi")));
        assert_eq!(Quantity::default(), Quantity::ZERO);
        assert_eq!(Quantity::from_value(3), q("3"));
        assert_eq!(Quantity::from_milli(-250), q("-250m"));
    }

    fn arb_format() -> impl Strategy<Value = QuantityFormat> {
        prop_oneof![
            Just(QuantityFormat::BinarySI),
            Just(QuantityFormat::DecimalSI),
            Just(QuantityFormat::DecimalExponent),
        ]
    }

    proptest! {
        #[test]
        fn canonical_form_round_trips(nanos in any::<i128>(), format in arb_format()) {
            let original = Quantity::from_nanos(nanos).with_format(format);
            let text = original.to_string();
            let parsed = Quantity::parse(&text).unwrap();
            prop_assert_eq!(parsed, original);
            prop_assert_eq!(parsed.to_string(), text);
        }

        #[test]
        fn small_values_round_trip(nanos in -10_000_000_000_000i128..10_000_000_000_000, format in arb_format()) {
            let original = Quantity::from_nanos(nanos).with_format(format);
            prop_assert_eq!(Quantity::parse(&original.to_string()).unwrap(), original);
        }

        #[test]
        fn generated_valid_strings_parse_to_expected_value(
            int in 0u64..1_000_000,
            frac in 0u32..1000,
            idx in 0usize..16,
            negative in any::<bool>(),
        ) {
            let suffixes: [(&str, i128); 16] = [
                ("n", 1), ("u", 1_000), ("m", 1_000_000), ("", NANO), ("k", NANO * 1_000),
                ("M", NANO * 1_000_000), ("G", NANO * 1_000_000_000), ("T", NANO * 1_000_000_000_000),
                ("P", NANO * 1_000_000_000_000_000), ("E", NANO * 1_000_000_000_000_000_000),
                ("Ki", NANO << 10), ("Mi", NANO << 20), ("Gi", NANO << 30), ("Ti", NANO << 40),
                ("Pi", NANO << 50), ("Ei", NANO << 60),
            ];
            let (suffix, unit) = suffixes[idx];
            let sign = if negative { "-" } else { "" };
            let text = format!("{sign}{int}.{frac:03}{suffix}");
            let parsed = Quantity::parse(&text).unwrap();
            // int.frac3 = (int * 1000 + frac) / 1000 ; nano-units are exact when unit is large
            // enough, otherwise rounded away from zero.
            let scaled = (i128::from(int) * 1000 + i128::from(frac)) * unit;
            let mag = scaled / 1000 + i128::from(scaled % 1000 != 0);
            prop_assert_eq!(parsed.nanos(), if negative { -mag } else { mag });
        }

        #[test]
        fn add_then_sub_restores(a in -(1i128 << 100)..(1i128 << 100), b in -(1i128 << 100)..(1i128 << 100)) {
            let (a, b) = (Quantity::from_nanos(a), Quantity::from_nanos(b));
            prop_assert_eq!((a + b) - b, a);
            prop_assert_eq!(a + b, b + a);
        }

        #[test]
        fn ordering_matches_nanos(a in any::<i128>(), b in any::<i128>()) {
            prop_assert_eq!(
                Quantity::from_nanos(a).cmp(&Quantity::from_nanos(b)),
                a.cmp(&b)
            );
        }

        #[test]
        fn parse_never_panics(s in "\\PC{0,24}") {
            let _ = Quantity::parse(&s);
        }

        #[test]
        fn parse_never_panics_on_quantity_like_input(s in "[-+.0-9eEkKmMGTPnuiB]{0,30}") {
            let _ = Quantity::parse(&s);
        }

        #[test]
        fn human_formatters_never_panic(nanos in any::<i128>()) {
            let q = Quantity::from_nanos(nanos);
            let _ = q.human_bytes();
            let _ = q.human_cpu();
            let _ = q.as_f64();
        }
    }
}
