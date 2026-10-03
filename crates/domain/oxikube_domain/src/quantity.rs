// Portions derived from kubectl-view-allocations `qty::Qty`
// (https://github.com/davidB/kubectl-view-allocations), released under CC0-1.0.
// The scale table and the "value plus scale" shape follow that parser; the stored value is an
// exact decimal here (no `f64`), and the grammar, rounding, canonical formatting and the reuse of
// canonical input text follow Kubernetes apimachinery `resource.Quantity` (Apache-2.0, semantics
// only; see `tests/quantity_corpus` for the ported test tables).
// Modifications (c) Oxikube contributors.

//! Exact Kubernetes resource quantities (`500m`, `1.5Gi`, `129e6`, `1e1000`).
//!
//! A [`Quantity`] stores its value as an exact decimal, `mantissa × 10^exponent`, with an `i128`
//! mantissa and an `i32` exponent. `0.1 + 0.2` style errors cannot appear, every value the
//! Kubernetes API can express with up to nano precision is exact (`1n` to `1e1000` and beyond),
//! and the representation is `Copy` and allocation-free, so table cells stay cheap. Values with
//! more than nine fractional digits are rounded **away from zero** to the next nano, matching
//! apimachinery (`1.5n` parses as `2n`, `0.000000000001Ki` as `2n`).
//!
//! Parsing follows apimachinery `ParseQuantity`: an optional sign, digits with an optional
//! fraction, then a suffix. An empty number is zero (`.`, `-`, `+`, `-.` and even `Gi` parse as
//! `0`, as they do in Go).
//!
//! [`Display`](std::fmt::Display) matches apimachinery `Quantity.String()`: inputs that Go
//! already considers canonical are echoed verbatim (`1.G`, `100.035k`, `1E6`, `+1Ki`), everything
//! else is printed in canonical form (`1024Mi` prints `1Gi`, `0.5` prints `500m`).
//!
//! Intentional differences from Go, all on values Go cannot round-trip:
//!
//! - No int64 saturation: Go clamps `BinarySI` values above `2^63 - 1` (`9Ei`, `10Ei`); a
//!   `Quantity` keeps the exact value and prints `9Ei` (apimachinery TODO #141166).
//! - `DecimalSI` values past the `E` suffix print with an exponent (`1000E` prints `1e21`);
//!   Go drops the suffix and prints `1`. `BinarySI` values past `Ei` keep the `Ei` suffix.
//! - Range: at most 38 significant digits after nano rounding (`i128` mantissa) and exponents up
//!   to [`Quantity::MAX_EXPONENT`]; anything larger is [`QuantityError::OutOfRange`]. Go keeps
//!   arbitrary precision and truncates exponents to `i32`.
//! - Go echoes canonical input of any length; inputs longer than 26 bytes (only possible with
//!   padding zeros) print canonically here.
//!
//! Floating point appears only in display helpers ([`Quantity::as_f64`],
//! [`Quantity::percent_of`], [`Quantity::human_bytes`]); never compare or sum those results.
//!
//! Equality, ordering and hashing look at the numeric value only: `1Gi == 1024Mi` and
//! `1000m == 1`. The [`QuantityFormat`] and the remembered input text only steer
//! [`Display`](std::fmt::Display).
//!
//! # Performance
//!
//! Parsing does not allocate (errors allocate, and so does the rare binary input with more than
//! about twenty significant digits). Canonical formatting writes straight into the formatter.
//! Quantities are `Copy`, so metrics samples can be recomputed per tick cheaply.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;

/// Exponent of the smallest representable step (one nano-unit).
const NANO_EXP: i32 = -9;

/// Longest input text a quantity remembers for verbatim display.
const SPELLING_CAP: usize = 26;

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

/// Why a string is not a valid [`Quantity`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuantityError {
    /// The input was empty.
    #[error("quantity is empty")]
    Empty,
    /// The numeric part is malformed (`"--1"`, `"1.2.3"`, `"1+1"`).
    #[error("invalid number in quantity {0:?}")]
    InvalidNumber(String),
    /// The suffix is not a Kubernetes quantity suffix or exponent (`"Gb"`, `"ki"`, `"e3.5"`).
    #[error("invalid suffix {0:?} in quantity")]
    InvalidSuffix(String),
    /// The value needs more than 38 significant digits at nano precision, or an exponent beyond
    /// [`Quantity::MAX_EXPONENT`].
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
#[derive(Clone, Copy, Default)]
pub struct Quantity {
    /// Significand; never ends in a zero digit unless it is zero.
    mantissa: i128,
    /// Power of ten; at least `-9`, zero for the zero value.
    exponent: i32,
    /// Notation used by `Display`.
    format: QuantityFormat,
    /// Length of `spelling`; zero when the canonical form is printed.
    spelling_len: u8,
    /// Input text apimachinery reuses as `String()` (ASCII).
    spelling: [u8; SPELLING_CAP],
}

/// A value as sign, magnitude and exponent, for exact arithmetic.
type Parts = (bool, u128, i64);

/// A parsed suffix.
struct Unit {
    format: QuantityFormat,
    /// Binary suffix as a power of two (`Ki` is 10).
    shift: u32,
    /// Decimal suffix or exponent as a power of ten.
    exp: i64,
}

impl Quantity {
    /// The zero quantity in `DecimalSI`.
    pub const ZERO: Quantity = Quantity::raw(0, 0, QuantityFormat::DecimalSI);

    /// Largest decimal exponent a quantity carries (`value = mantissa × 10^exponent`).
    pub const MAX_EXPONENT: i32 = 1_000_000_000;

    /// The largest quantity, `i128::MAX × 10^MAX_EXPONENT`; `+` and `-` saturate to it.
    pub const MAX: Quantity =
        Quantity::raw(i128::MAX, Self::MAX_EXPONENT, QuantityFormat::DecimalSI);

    /// The smallest quantity, `i128::MIN × 10^MAX_EXPONENT`; `+` and `-` saturate to it.
    pub const MIN: Quantity =
        Quantity::raw(i128::MIN, Self::MAX_EXPONENT, QuantityFormat::DecimalSI);

    const fn raw(mantissa: i128, exponent: i32, format: QuantityFormat) -> Self {
        Self {
            mantissa,
            exponent,
            format,
            spelling_len: 0,
            spelling: [0; SPELLING_CAP],
        }
    }

    /// `mantissa × 10^exponent` in `DecimalSI`, normalised (exponent must be small).
    const fn from_scaled(mut mantissa: i128, mut exponent: i32) -> Self {
        if mantissa == 0 {
            return Self::ZERO;
        }
        while mantissa % 10 == 0 {
            mantissa /= 10;
            exponent += 1;
        }
        Self::raw(mantissa, exponent, QuantityFormat::DecimalSI)
    }

    /// Builds a quantity from a raw count of nano-units.
    pub const fn from_nanos(nanos: i128) -> Self {
        Self::from_scaled(nanos, NANO_EXP)
    }

    /// Builds a quantity from milli-units (`1500` is `1500m`, one and a half cores).
    pub const fn from_milli(milli: i64) -> Self {
        Self::from_scaled(milli as i128, -3)
    }

    /// Builds a quantity from whole units (bytes, cores, pods).
    pub const fn from_value(value: i64) -> Self {
        Self::from_scaled(value as i128, 0)
    }

    /// Returns the same value with a different canonical notation (and forgets the input text).
    #[must_use]
    pub const fn with_format(mut self, format: QuantityFormat) -> Self {
        self.format = format;
        self.spelling_len = 0;
        self
    }

    /// The notation family this quantity prints in.
    pub const fn format(&self) -> QuantityFormat {
        self.format
    }

    /// The significand: the value is `mantissa() × 10^exponent()`. It has no trailing zero
    /// digits (zero is `0 × 10^0`), so equal values have equal parts.
    pub const fn mantissa(&self) -> i128 {
        self.mantissa
    }

    /// The power of ten, at least `-9` (one nano). See [`Quantity::mantissa`].
    pub const fn exponent(&self) -> i32 {
        self.exponent
    }

    /// Parses a Kubernetes quantity string the way apimachinery `ParseQuantity` does.
    ///
    /// Accepts an optional sign, digits with an optional fraction (`1.5`, `.5`, `1.`; an empty
    /// number is zero), and one of: no suffix, a decimal suffix (`n u m k M G T P E`), a binary
    /// suffix (`Ki Mi Gi Ti Pi Ei`) or an exponent (`1e3`, `12E6`, `5e-3`, `1e+3`). Whitespace
    /// is not allowed. A binary value between zero and one switches to `DecimalSI`, as in Go.
    ///
    /// # Errors
    ///
    /// Returns a [`QuantityError`] for empty input, a malformed number, an unknown suffix, or a
    /// value outside the representable range.
    pub fn parse(input: &str) -> Result<Self, QuantityError> {
        if input.is_empty() {
            return Err(QuantityError::Empty);
        }
        let bytes = input.as_bytes();
        let negative = bytes[0] == b'-';
        let mut i = usize::from(matches!(bytes[0], b'+' | b'-'));
        let int_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let int = &bytes[int_start..i];
        let mut frac: &[u8] = &[];
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
            let frac_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            frac = &bytes[frac_start..i];
        }
        let suffix = &input[i..];
        let Some(unit) = parse_suffix(suffix) else {
            return Err(if suffix.starts_with(['.', '+', '-']) {
                QuantityError::InvalidNumber(input.to_owned())
            } else {
                QuantityError::InvalidSuffix(suffix.to_owned())
            });
        };
        // Go reads an empty number as zero only on its fast path; its big-decimal fallback
        // (exponents below -9, `Pi` and `Ei`) rejects it.
        let fast_path = match unit.format {
            QuantityFormat::BinarySI => unit.shift <= 40,
            _ => unit.exp >= i64::from(NANO_EXP),
        };
        if int.is_empty() && frac.is_empty() && !fast_path {
            return Err(QuantityError::InvalidNumber(input.to_owned()));
        }
        let digits = Digits { int, frac };
        let out_of_range = || QuantityError::OutOfRange(input.to_owned());

        // value = digits × 2^shift × 10^(exp - frac_len), rounded away from zero to a nano.
        let frac_len = i64::try_from(frac.len()).unwrap_or(i64::MAX);
        let exp10 = unit.exp.saturating_sub(frac_len);
        let (magnitude, exponent) = if unit.shift == 0 {
            round_to_nano(digits, exp10)
        } else {
            binary_magnitude(digits, unit.shift, exp10)
        }
        .ok_or_else(out_of_range)?;

        let mut format = unit.format;
        if format == QuantityFormat::BinarySI && magnitude != 0 && below_one(magnitude, exponent) {
            format = QuantityFormat::DecimalSI;
        }
        let mut quantity =
            Self::from_parts((negative, magnitude, exponent), format).ok_or_else(out_of_range)?;
        if input.len() <= SPELLING_CAP && go_reuses_input(digits, &unit) {
            quantity.spelling[..input.len()].copy_from_slice(bytes);
            quantity.spelling_len = input.len() as u8;
        }
        Ok(quantity)
    }

    /// The exact value in nano-units, or `None` when it does not fit in an `i128`.
    pub const fn checked_nanos(&self) -> Option<i128> {
        let shift = self.exponent - NANO_EXP;
        if shift > 38 {
            return None;
        }
        self.mantissa.checked_mul(10i128.pow(shift as u32))
    }

    /// The exact value in nano-units, saturating at `i128::MIN` / `i128::MAX` for values beyond
    /// about `1.7e29`. Use [`Quantity::checked_nanos`] to detect that.
    pub const fn nanos(&self) -> i128 {
        match self.checked_nanos() {
            Some(nanos) => nanos,
            None if self.mantissa < 0 => i128::MIN,
            None => i128::MAX,
        }
    }

    /// The value in whole units, rounded away from zero like apimachinery `Value()` (`1500m` is
    /// `2`, `-1500m` is `-2`). Saturates at the `i128` bounds.
    pub fn value(&self) -> i128 {
        self.rounded_at(0)
    }

    /// The value in milli-units, rounded away from zero like apimachinery `MilliValue()`
    /// (`1500u` is `2`, `-1500u` is `-2`, `1Gi` is `1073741824000`). Saturates at the `i128`
    /// bounds.
    pub fn milli_value(&self) -> i128 {
        self.rounded_at(-3)
    }

    /// `self / 10^exp`, rounded away from zero, saturating.
    fn rounded_at(&self, exp: i32) -> i128 {
        let shift = self.exponent - exp;
        if shift < 0 {
            return div_away_from_zero(self.mantissa, 10i128.pow(shift.unsigned_abs()));
        }
        let saturated = if self.mantissa < 0 {
            i128::MIN
        } else {
            i128::MAX
        };
        u32::try_from(shift)
            .ok()
            .and_then(|s| 10i128.checked_pow(s))
            .and_then(|p| self.mantissa.checked_mul(p))
            .unwrap_or(saturated)
    }

    /// Whether the value is exactly zero.
    pub const fn is_zero(&self) -> bool {
        self.mantissa == 0
    }

    /// Whether the value is below zero.
    pub const fn is_negative(&self) -> bool {
        self.mantissa < 0
    }

    /// The value as a float. **For display only**: it loses precision beyond 2^53 and is
    /// infinite past `f64::MAX`; never use it for comparison or arithmetic.
    pub fn as_f64(&self) -> f64 {
        if self.mantissa == 0 {
            return 0.0;
        }
        let m = self.mantissa as f64;
        if self.exponent >= 0 {
            m * 10f64.powi(self.exponent)
        } else {
            m / 10f64.powi(-self.exponent)
        }
    }

    /// Utilisation of `self` against `total` in percent (`250m` of `1` is `25.0`), or `None`
    /// when `total` is zero. The result is a float for display only.
    pub fn percent_of(&self, total: &Quantity) -> Option<f64> {
        if total.mantissa == 0 {
            return None;
        }
        let mut num = self.mantissa as f64 * 100.0;
        let mut den = total.mantissa as f64;
        // Scale the side that keeps small powers of ten exact.
        let shift = i64::from(self.exponent) - i64::from(total.exponent);
        match shift {
            0 => {}
            1..=22 => num *= 10f64.powi(shift as i32),
            -22..=-1 => den *= 10f64.powi((-shift) as i32),
            _ => {
                let shift = shift.clamp(-400, 400) as i32;
                return Some(num / den * 10f64.powi(shift));
            }
        }
        Some(num / den)
    }

    /// Sum, or `None` when the exact result is not representable. The result keeps `self`'s
    /// notation (the other operand's when `self` is zero).
    pub fn checked_add(&self, other: &Quantity) -> Option<Quantity> {
        let parts = add_parts(self.parts(), other.parts())?;
        Self::from_parts(parts, self.format_for_sum(other))
    }

    /// Difference, or `None` when the exact result is not representable. Notation follows
    /// [`Quantity::checked_add`].
    pub fn checked_sub(&self, other: &Quantity) -> Option<Quantity> {
        let parts = add_parts(self.parts(), other.negated_parts())?;
        Self::from_parts(parts, self.format_for_sum(other))
    }

    fn format_for_sum(&self, other: &Quantity) -> QuantityFormat {
        if self.mantissa == 0 {
            other.format
        } else {
            self.format
        }
    }

    /// `self + other` or, when not representable, [`Quantity::MAX`] / [`Quantity::MIN`]
    /// following the sign of the exact result.
    fn saturating_sum(&self, other: Parts, format: QuantityFormat) -> Quantity {
        let a = self.parts();
        match add_parts(a, other).and_then(|parts| Self::from_parts(parts, format)) {
            Some(sum) => sum,
            None => {
                let negative = match (a.0 == other.0, cmp_magnitude(a, other)) {
                    (true, _) | (false, Ordering::Greater) => a.0,
                    (false, _) => other.0,
                };
                let bound = if negative { Self::MIN } else { Self::MAX };
                bound.with_format(format)
            }
        }
    }

    fn parts(&self) -> Parts {
        (
            self.mantissa < 0,
            self.mantissa.unsigned_abs(),
            i64::from(self.exponent),
        )
    }

    fn negated_parts(&self) -> Parts {
        let (negative, magnitude, exponent) = self.parts();
        (!negative && magnitude != 0, magnitude, exponent)
    }

    /// Normalises a sign/magnitude/exponent triple, or `None` when it is out of range.
    fn from_parts(
        (negative, mut magnitude, mut exponent): Parts,
        format: QuantityFormat,
    ) -> Option<Self> {
        if magnitude == 0 {
            return Some(Self::ZERO.with_format(format));
        }
        while magnitude % 10 == 0 {
            magnitude /= 10;
            exponent += 1;
        }
        if !(i64::from(NANO_EXP)..=i64::from(Self::MAX_EXPONENT)).contains(&exponent) {
            return None;
        }
        let mantissa = if negative {
            0i128.checked_sub_unsigned(magnitude)?
        } else {
            i128::try_from(magnitude).ok()?
        };
        Some(Self::raw(mantissa, exponent as i32, format))
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
        let sign = if self.is_negative() && rounded > 0.0 {
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
    /// (`250m`, `1500m`, `0`). Sub-millicore values round away from zero.
    pub fn human_cpu(&self) -> String {
        match self.milli_value() {
            0 => "0".to_owned(),
            m => format!("{m}m"),
        }
    }

    /// The remembered input text, if apimachinery would reuse it as `String()`.
    fn spelling(&self) -> Option<&str> {
        if self.spelling_len == 0 {
            return None;
        }
        std::str::from_utf8(&self.spelling[..usize::from(self.spelling_len)]).ok()
    }

    /// `|value|` and its power-of-1024 suffix index when apimachinery prints this `BinarySI`
    /// value with a binary suffix: an integer of magnitude 1024 or more.
    fn binary_form(&self) -> Option<(u128, usize)> {
        let exponent = u32::try_from(self.exponent).ok()?;
        let mut v = self
            .mantissa
            .unsigned_abs()
            .checked_mul(10u128.checked_pow(exponent)?)?;
        if v < 1024 {
            return None;
        }
        let mut k = 0;
        while k < BINARY_SUFFIXES.len() && v % 1024 == 0 {
            v /= 1024;
            k += 1;
        }
        Some((v, k))
    }

    /// Writes the canonical Kubernetes text.
    fn write_canonical(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mantissa == 0 {
            return f.write_str("0");
        }
        if self.format == QuantityFormat::BinarySI {
            if let Some((v, k)) = self.binary_form() {
                let sign = if self.mantissa < 0 { "-" } else { "" };
                let suffix = if k == 0 { "" } else { BINARY_SUFFIXES[k - 1] };
                return write!(f, "{sign}{v}{suffix}");
            }
        }
        // apimachinery: strip factors of ten, then lower the exponent to a multiple of three.
        let e3 = self.exponent.div_euclid(3) * 3;
        let zeros = &"00"[..(self.exponent - e3) as usize];
        let m = self.mantissa;
        let si = DECIMAL_SUFFIXES.iter().find(|(_, exp)| *exp == e3);
        match si {
            Some((suffix, _)) if self.format != QuantityFormat::DecimalExponent => {
                write!(f, "{m}{zeros}{suffix}")
            }
            _ if e3 == 0 => write!(f, "{m}{zeros}"),
            _ => write!(f, "{m}{zeros}e{e3}"),
        }
    }
}

/// The digits of a number, split around the decimal point, as ASCII.
#[derive(Clone, Copy)]
struct Digits<'a> {
    int: &'a [u8],
    frac: &'a [u8],
}

impl Digits<'_> {
    fn len(&self) -> usize {
        self.int.len() + self.frac.len()
    }

    /// The digit value at position `i` of `int ++ frac`.
    fn at(&self, i: usize) -> u8 {
        match self.int.get(i) {
            Some(b) => b - b'0',
            None => self.frac[i - self.int.len()] - b'0',
        }
    }

    /// The range of significant digits (no leading or trailing zeros); empty when zero.
    fn significant(&self) -> std::ops::Range<usize> {
        let len = self.len();
        let start = (0..len).take_while(|&i| self.at(i) == 0).count();
        if start == len {
            return 0..0;
        }
        let trailing = (start..len).rev().take_while(|&i| self.at(i) == 0).count();
        start..len - trailing
    }

    fn accumulate(&self, range: std::ops::Range<usize>) -> Option<u128> {
        range.into_iter().try_fold(0u128, |acc, i| {
            acc.checked_mul(10)?.checked_add(u128::from(self.at(i)))
        })
    }
}

/// `digits × 10^exp10` rounded away from zero to a nano, as `(magnitude, exponent)` with
/// `exponent >= -9`, or `None` if the magnitude does not fit in a `u128`.
fn round_to_nano(digits: Digits<'_>, exp10: i64) -> Option<(u128, i64)> {
    let sig = digits.significant();
    if sig.is_empty() {
        return Some((0, 0));
    }
    let exp = exp10.saturating_add((digits.len() - sig.end) as i64);
    let nano = i64::from(NANO_EXP);
    if exp >= nano {
        return Some((digits.accumulate(sig)?, exp));
    }
    // Drop the digits below one nano; the last significant digit is non-zero, so anything
    // dropped makes the result round up.
    let cut = nano.saturating_sub(exp);
    let keep = (sig.len() as i64).saturating_sub(cut);
    if keep <= 0 {
        return Some((1, nano));
    }
    // Round up the last kept digit; a carry out of it leaves a trailing zero, which keeps
    // 39-digit intermediates such as `340…9|9` representable.
    let last = sig.start + keep as usize - 1;
    let prefix = digits.accumulate(sig.start..last)?;
    match digits.at(last) + 1 {
        10 => Some((prefix.checked_add(1)?, nano + 1)),
        d => Some((prefix.checked_mul(10)?.checked_add(u128::from(d))?, nano)),
    }
}

/// `digits × 2^shift × 10^exp10`, rounded like [`round_to_nano`].
fn binary_magnitude(digits: Digits<'_>, shift: u32, exp10: i64) -> Option<(u128, i64)> {
    let sig = digits.significant();
    if sig.is_empty() {
        return Some((0, 0));
    }
    let exp = exp10.saturating_add((digits.len() - sig.end) as i64);
    let fast = digits
        .accumulate(sig.clone())
        .and_then(|d| d.checked_mul(1u128 << shift));
    if let Some(v) = fast {
        let nano = i64::from(NANO_EXP);
        if exp >= nano {
            return Some((v, exp));
        }
        let cut = nano.saturating_sub(exp);
        if cut > 38 {
            return Some((1, nano));
        }
        let unit = 10u128.pow(cut as u32);
        return Some((v / unit + u128::from(v % unit != 0), nano));
    }
    // Rare: too many significant digits for u128 arithmetic. Multiply in decimal instead.
    let mut buf: Vec<u8> = sig.map(|i| b'0' + digits.at(i)).collect();
    let mut carry = 0u128;
    for b in buf.iter_mut().rev() {
        let t = u128::from(*b - b'0') * (1u128 << shift) + carry;
        *b = b'0' + (t % 10) as u8;
        carry = t / 10;
    }
    let mut head = Vec::new();
    while carry > 0 {
        head.push(b'0' + (carry % 10) as u8);
        carry /= 10;
    }
    head.reverse();
    head.extend_from_slice(&buf);
    round_to_nano(
        Digits {
            int: &head,
            frac: &[],
        },
        exp,
    )
}

/// Whether `magnitude × 10^exponent < 1` (`exponent >= -9`).
fn below_one(magnitude: u128, exponent: i64) -> bool {
    exponent < 0 && magnitude < 10u128.pow(exponent.unsigned_abs() as u32)
}

/// Whether apimachinery `ParseQuantity` keeps the input as the `String()` value: its fast
/// path for at most 18 digits, when the text is already in canonical shape.
fn go_reuses_input(digits: Digits<'_>, unit: &Unit) -> bool {
    let first_non_zero = digits.int.iter().position(|b| *b != b'0');
    let num = first_non_zero.map_or(&[][..], |p| &digits.int[p..]);
    let denom = digits.frac;
    // Go treats an empty integer part as "0".
    let num_len = num.len().max(1) as i64;
    if unit.format == QuantityFormat::BinarySI {
        let precision = 15 - num_len - i64::from(unit.shift) * 3 / 10 - 1;
        if !denom.is_empty() || precision < 0 {
            return false;
        }
        let value = num
            .iter()
            .fold(0u64, |acc, b| acc * 10 + u64::from(b - b'0'));
        return value & 0x07 != 0;
    }
    let denom_len = denom.len() as i64;
    if num.is_empty() || num_len + denom_len > 18 {
        return false;
    }
    let scale = unit.exp - denom_len;
    let trailing_zeros = num
        .iter()
        .chain(denom)
        .rev()
        .take(3)
        .filter(|b| **b == b'0')
        .count();
    let ends_in_000 = num_len + denom_len >= 3 && trailing_zeros == 3;
    scale >= i64::from(NANO_EXP) && scale % 3 == 0 && !ends_in_000
}

/// Interprets a suffix like apimachinery `quantitySuffixer.interpret`.
fn parse_suffix(suffix: &str) -> Option<Unit> {
    if let Some(k) = BINARY_SUFFIXES.iter().position(|s| *s == suffix) {
        return Some(Unit {
            format: QuantityFormat::BinarySI,
            shift: 10 * (k as u32 + 1),
            exp: 0,
        });
    }
    if let Some((_, exp)) = DECIMAL_SUFFIXES.iter().find(|(s, _)| *s == suffix) {
        return Some(Unit {
            format: QuantityFormat::DecimalSI,
            shift: 0,
            exp: i64::from(*exp),
        });
    }
    // `E` alone is exa (handled above); `E`/`e` followed by a signed int64 is an exponent.
    let rest = suffix.strip_prefix(['e', 'E'])?;
    let (negative, digits) = match rest.as_bytes().first()? {
        b'+' => (false, &rest[1..]),
        b'-' => (true, &rest[1..]),
        _ => (false, rest),
    };
    if digits.is_empty() {
        return None;
    }
    let mut exp: i64 = 0;
    for b in digits.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        let d = i64::from(b - b'0');
        exp = exp.checked_mul(10)?;
        exp = if negative {
            exp.checked_sub(d)?
        } else {
            exp.checked_add(d)?
        };
    }
    Some(Unit {
        format: QuantityFormat::DecimalExponent,
        shift: 0,
        exp,
    })
}

/// Compares `|a|` with `|b|`.
fn cmp_magnitude(a: Parts, b: Parts) -> Ordering {
    let (_, am, ae) = a;
    let (_, bm, be) = b;
    match (am == 0, bm == 0) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        (false, false) => {}
    }
    let a_order = i64::from(am.ilog10()) + ae;
    let b_order = i64::from(bm.ilog10()) + be;
    if a_order != b_order {
        return a_order.cmp(&b_order);
    }
    // Same order of magnitude, so the exponents differ by at most 38.
    let scale = |m: u128, by: i64| m.checked_mul(10u128.pow(by as u32));
    match ae.cmp(&be) {
        Ordering::Equal => am.cmp(&bm),
        Ordering::Greater => scale(am, ae - be).map_or(Ordering::Greater, |a| a.cmp(&bm)),
        Ordering::Less => scale(bm, be - ae).map_or(Ordering::Less, |b| am.cmp(&b)),
    }
}

/// Exact `a + b`, or `None` when the magnitude cannot be represented.
///
/// Mantissas are normalised (no trailing zero digit), so when the exponents differ the
/// result ends in a non-zero digit at the smaller exponent: if aligning overflows, so does
/// the result.
fn add_parts(a: Parts, b: Parts) -> Option<Parts> {
    if a.1 == 0 {
        return Some(b);
    }
    if b.1 == 0 {
        return Some(a);
    }
    let (hi, lo) = if a.2 >= b.2 { (a, b) } else { (b, a) };
    let shift = u32::try_from(hi.2 - lo.2).ok()?;
    let hi_mag = hi.1.checked_mul(10u128.checked_pow(shift)?)?;
    if hi.0 == lo.0 {
        // Equal exponents can overflow u128 only at 2^128, which no quantity can hold.
        return Some((hi.0, hi_mag.checked_add(lo.1)?, lo.2));
    }
    Some(if hi_mag >= lo.1 {
        (hi.0, hi_mag - lo.1, lo.2)
    } else {
        (lo.0, lo.1 - hi_mag, lo.2)
    })
}

/// Division rounding away from zero, as apimachinery does for `Value()` and `MilliValue()`
/// (`1500m` is `2`, `-1500m` is `-2`).
fn div_away_from_zero(a: i128, b: i128) -> i128 {
    let q = a / b;
    if a % b == 0 {
        q
    } else if (a > 0) == (b > 0) {
        q + 1
    } else {
        q - 1
    }
}

impl FromStr for Quantity {
    type Err = QuantityError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// The text apimachinery `Quantity.String()` produces: the input itself when Go considers it
/// canonical (`1.G`, `+1Ki`), otherwise the largest suffix of the quantity's
/// [`QuantityFormat`] that keeps the mantissa an integer (`1536Mi`, `1500m`, `1k`, `12e6`).
/// `parse(q.to_string()) == q`.
impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.spelling() {
            Some(text) => f.write_str(text),
            None => self.write_canonical(f),
        }
    }
}

impl fmt::Debug for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Quantity")
            .field("mantissa", &self.mantissa)
            .field("exponent", &self.exponent)
            .field("format", &self.format)
            .field("text", &format_args!("{self}"))
            .finish()
    }
}

impl PartialEq for Quantity {
    fn eq(&self, other: &Self) -> bool {
        self.mantissa == other.mantissa && self.exponent == other.exponent
    }
}

impl Eq for Quantity {}

impl Hash for Quantity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.mantissa.hash(state);
        self.exponent.hash(state);
    }
}

impl PartialOrd for Quantity {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Quantity {
    fn cmp(&self, other: &Self) -> Ordering {
        let (a, b) = (self.mantissa.signum(), other.mantissa.signum());
        if a != b {
            return a.cmp(&b);
        }
        let magnitude = cmp_magnitude(self.parts(), other.parts());
        if a < 0 {
            magnitude.reverse()
        } else {
            magnitude
        }
    }
}

/// Exact addition that saturates at [`Quantity::MAX`] / [`Quantity::MIN`] when the result is
/// not representable; use [`Quantity::checked_add`] to detect that.
impl Add for Quantity {
    type Output = Quantity;

    fn add(self, rhs: Quantity) -> Quantity {
        self.saturating_sum(rhs.parts(), self.format_for_sum(&rhs))
    }
}

/// Exact subtraction that saturates like [`Add`]; use [`Quantity::checked_sub`] to detect
/// overflow.
impl Sub for Quantity {
    type Output = Quantity;

    fn sub(self, rhs: Quantity) -> Quantity {
        self.saturating_sum(rhs.negated_parts(), self.format_for_sum(&rhs))
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

    /// Nano-units per whole unit.
    const NANO: i128 = 1_000_000_000;

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
        // Far beyond the old i128 nano range, still exact.
        assert_eq!((q("1e30").mantissa(), q("1e30").exponent()), (1, 30));
        assert_eq!((q("1e1000").mantissa(), q("1e1000").exponent()), (1, 1000));
        assert_eq!(q("1e1000").checked_nanos(), None);
        assert_eq!(q("1e1000").nanos(), i128::MAX);
        assert_eq!(q("-1e1000").nanos(), i128::MIN);
        assert_eq!(q("1e1000").value(), i128::MAX);
        assert_eq!(q("-1e1000").milli_value(), i128::MIN);
        assert_eq!(q("1e1000").to_string(), "10e999");
        assert!(q("1e1000") > q("9e999"));
        assert!(q("-1e1000") < q("-9e999"));
        let huge = format!("{}e{}", i128::MAX, Quantity::MAX_EXPONENT);
        assert_eq!(q(&huge), Quantity::MAX);
        for out_of_range in [
            "1e99999999999".to_owned(),
            "9".repeat(60),
            format!("1e{}", i64::from(Quantity::MAX_EXPONENT) + 1),
            u128::MAX.to_string(),
        ] {
            assert!(
                matches!(
                    Quantity::parse(&out_of_range),
                    Err(QuantityError::OutOfRange(_))
                ),
                "{out_of_range}"
            );
        }
        // More than 38 digits is fine when the extra ones are trailing zeros or below a nano.
        assert_eq!(q(&format!("1{}", "0".repeat(60))).exponent(), 60);
        assert_eq!(q(&format!("0.{}1", "0".repeat(60))).nanos(), 1);
        assert_eq!(q(&format!("1.{}1", "0".repeat(60))).nanos(), NANO + 1);
        // Rounding a 39-digit prefix that overflows u128 until the carry adds a trailing zero.
        let carry = q("+341903040034170874288692318.7956182231199k");
        assert_eq!(
            (carry.mantissa(), carry.exponent()),
            (34_190_304_003_417_087_428_869_231_879_561_822_312, -8)
        );
        assert!(matches!(
            Quantity::parse("341903040034170874288692318.7956182231181k"),
            Err(QuantityError::OutOfRange(_))
        ));
        // A binary value with many significant digits takes the decimal slow path.
        let long_bin = format!("0.{}Ki", "1".repeat(45));
        assert_eq!(q(&long_bin).nanos(), 113_777_777_778);
        let long_int_bin = format!("{}Ki", "1".repeat(30));
        assert_eq!(
            (q(&long_int_bin).mantissa(), q(&long_int_bin).exponent()),
            (113_777_777_777_777_777_777_777_777_777_664, 0)
        );
        let too_long_bin = format!("{}Ki", "1".repeat(40));
        assert!(matches!(
            Quantity::parse(&too_long_bin),
            Err(QuantityError::OutOfRange(_))
        ));
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(Quantity::parse(""), Err(QuantityError::Empty));
        for s in [
            "1Gb",
            "1.2.3",
            "--1",
            "1 Gi",
            " 1",
            "1 ",
            "1ki",
            "1gi",
            "1K",
            "1g",
            "1e",
            "1ee3",
            "1e3.5",
            "1e+",
            "1e-",
            "1Mib",
            "0x10",
            "NaN",
            "inf",
            "1,5",
            "1_000",
            "１",
            "1e1_0",
            "1e99999999999999999999",
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
        assert!(matches!(
            Quantity::parse("1.2.3"),
            Err(QuantityError::InvalidNumber(_))
        ));
    }

    #[test]
    fn empty_number_is_zero_like_go() {
        for s in [
            ".", "-", "+", "-.", "+.", "m", "Gi", "-Ki", "Ti", "e3", "e-9",
        ] {
            assert!(q(s).is_zero(), "{s}");
            assert_eq!(q(s).to_string(), "0", "{s}");
        }
        // Go's slow path needs at least one digit.
        for s in ["Pi", "Ei", ".Pi", "-.Ei", "e-10", ".e-20"] {
            assert!(
                matches!(Quantity::parse(s), Err(QuantityError::InvalidNumber(_))),
                "{s}"
            );
        }
        assert!(q("0Pi").is_zero());
        assert!(q("0.e-20").is_zero());
        assert_eq!(q("Gi").format(), QuantityFormat::BinarySI);
        assert_eq!(q("e3").format(), QuantityFormat::DecimalExponent);
    }

    #[test]
    fn reuses_canonical_input_text_like_go() {
        // Echoed verbatim: Go's fast path finds them already canonical.
        for s in [
            "1.", "1.G", "100.035k", "1E6", "1E-3", "+1", "+1Ki", "007", "007Ki", "1.500", "1e+3",
            "1e03", "1.Ki", "-1.G",
        ] {
            assert_eq!(q(s).to_string(), s);
        }
        // Rewritten: not canonical in Go's eyes.
        for (s, want) in [
            ("+8Ki", "8Ki"),
            ("+1Pi", "1Pi"),
            ("1e14", "100e12"),
            (".5", "500m"),
            ("1000M", "1G"),
            ("1024Mi", "1Gi"),
            ("0.5Mi", "512Ki"),
        ] {
            assert_eq!(q(s).to_string(), want, "{s}");
        }
        // Changing the notation or doing arithmetic drops the remembered text.
        let plus = q("+1Ki").with_format(QuantityFormat::BinarySI);
        assert_eq!(plus.to_string(), "1Ki");
        assert_eq!((q("1.G") + Quantity::ZERO).to_string(), "1G");
        // Canonical input longer than the spelling buffer prints canonically.
        let long = format!("{}7", "0".repeat(SPELLING_CAP - 1));
        assert_eq!(q(&long).to_string(), long);
        let longer = format!("{}7", "0".repeat(SPELLING_CAP));
        assert_eq!(q(&longer).to_string(), "7");
    }

    #[test]
    fn large_values_print_with_an_exponent_past_exa() {
        assert_eq!(q("1000E").to_string(), "1e21");
        assert_eq!(q("5000E").to_string(), "5e21");
        assert_eq!(q("1000000E").to_string(), "1e24");
        assert_eq!(q("-1000E").to_string(), "-1e21");
        assert_eq!(q("10E").to_string(), "10E");
        assert_eq!(q("1024Ei").to_string(), "1024Ei");
        let bin = |s: &str| q(s).with_format(QuantityFormat::BinarySI).to_string();
        assert_eq!(bin("1e21"), "953674316406250Mi");
        assert_eq!(bin("1e40"), "10e39");
        assert_eq!(bin("-1023"), "-1023");
        assert_eq!(bin("1023.5"), "1023500m");
        assert_eq!(bin("2048.5"), "2048500m");
    }

    #[test]
    fn binary_format_drops_to_decimal_below_one() {
        assert_eq!(q(".000001Ki").format(), QuantityFormat::DecimalSI);
        assert_eq!(q(".000000000001Ki").to_string(), "2n");
        assert_eq!(q("0.5Ki").format(), QuantityFormat::BinarySI);
        assert_eq!(q("0Ki").format(), QuantityFormat::BinarySI);
        assert_eq!(q("0.0009765624999Ki").format(), QuantityFormat::BinarySI);
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
            ("12E6", "12E6"),
            ("12000E3", "12e6"),
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
    fn value_helpers_round_away_from_zero() {
        assert_eq!(q("1500m").value(), 2);
        assert_eq!(q("-1500m").value(), -2);
        assert_eq!(q("-1500u").milli_value(), -2);
        assert_eq!(q("-1n").value(), -1);
        assert_eq!(q("-250m").human_cpu(), "-250m");
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
        // i128::MAX nanos + 1n is 2^127 nanos: a 39-digit mantissa ending in 8.
        let max_nanos = Quantity::from_nanos(i128::MAX);
        assert_eq!(max_nanos.checked_add(&q("1n")), None);
        assert_eq!(max_nanos + q("1n"), Quantity::MAX);
        let min_nanos = Quantity::from_nanos(i128::MIN);
        assert_eq!(min_nanos.checked_sub(&q("1n")), None);
        assert_eq!(min_nanos - q("1n"), Quantity::MIN);
        assert_eq!(Quantity::ZERO.checked_sub(&min_nanos), None);
        assert_eq!(min_nanos.checked_sub(&min_nanos), Some(Quantity::ZERO));
        // A carry that ends in zero stays exact past the i128 mantissa.
        let half = q("85070591730234615865843651857942052865");
        assert_eq!(
            half.checked_add(&half)
                .map(|s| (s.mantissa(), s.exponent())),
            Some((17_014_118_346_046_923_173_168_730_371_588_410_573, 1))
        );
        assert_eq!(
            half.checked_add(&q("85070591730234615865843651857942052863")),
            None
        );
        // Opposite signs where aligning overflows i128 but the result fits.
        let a = q("17014118346046923173168730371588410573e1");
        let b = q("-170141183460469231731687303715884105727");
        assert_eq!(a.checked_add(&b), Some(q("3")));
        // Wildly different exponents.
        assert_eq!(q("1e1000").checked_add(&q("1")), None);
        assert_eq!(q("1e1000") + q("1"), Quantity::MAX);
        assert_eq!(q("1") - q("1e1000"), Quantity::MIN);
        assert_eq!(q("1e1000") - q("2e1000"), q("-1e1000"));
        assert_eq!(q("1e1000").checked_sub(&q("1e1000")), Some(Quantity::ZERO));
        assert_eq!(Quantity::MAX + Quantity::MAX, Quantity::MAX);
        assert_eq!(Quantity::MIN + Quantity::MIN, Quantity::MIN);
        assert_eq!(Quantity::MAX + Quantity::MIN, q("-1e1000000000"));
        assert_eq!(q("1").checked_add(&q("1")), Some(q("2")));
        assert_eq!(q("3").checked_sub(&q("1")), Some(q("2")));
        assert_eq!(q("1e1000").percent_of(&q("1")), Some(f64::INFINITY));
        assert_eq!(q("1").percent_of(&q("1e1000")), Some(0.0));
        assert_eq!(q("1e1000").percent_of(&q("2e1000")), Some(50.0));
        assert_eq!(q("1e1000").as_f64(), f64::INFINITY);
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
        fn wide_range_round_trips(m in any::<i128>(), e in -9i32..3000, format in arb_format()) {
            let original = q(&format!("{m}e{e}")).with_format(format);
            let text = original.to_string();
            let parsed = Quantity::parse(&text).unwrap();
            prop_assert_eq!(parsed, original);
            prop_assert_eq!(parsed.to_string(), text);
        }

        #[test]
        fn wide_range_arithmetic_is_exact_or_saturates(
            (ma, ea) in (any::<i128>(), -9i32..60),
            (mb, eb) in (any::<i128>(), -9i32..60),
        ) {
            let a = q(&format!("{ma}e{ea}"));
            let b = q(&format!("{mb}e{eb}"));
            prop_assert_eq!(a + b, b + a);
            match a.checked_add(&b) {
                Some(sum) => {
                    prop_assert_eq!(a + b, sum);
                    prop_assert_eq!(sum.checked_sub(&b), Some(a));
                }
                None => prop_assert!(a + b == Quantity::MAX || a + b == Quantity::MIN),
            }
            match a.checked_sub(&b) {
                Some(diff) => {
                    prop_assert_eq!(diff.cmp(&Quantity::ZERO), a.cmp(&b));
                    prop_assert_eq!(diff.checked_add(&b), Some(a));
                }
                None => prop_assert!(a - b == Quantity::MAX || a - b == Quantity::MIN),
            }
            let _ = a.percent_of(&b);
            let _ = (a.value(), a.milli_value(), a.nanos(), a.as_f64(), a.human_bytes());
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
