//! [`Cell`]: what one table cell shows ([`CellValue::text`]), how it sorts ([`CellSort`]) and how
//! it should be coloured ([`Tone`]).
//!
//! A cell borrows its text from the object it was read from wherever it can (`Cow::Borrowed`),
//! so the common case of a string field costs no allocation. The sort key is typed, so a view can
//! order rows without parsing text again: `9` before `10`, `500Mi` before `2Gi`, `3m` before `2d`.
//! The tone is a meaning (ok / warn / error), never a colour: the table maps it to the `oxikube`
//! theme tokens.

use std::borrow::Cow;
use std::cmp::Ordering;

use jiff::Timestamp;
use oxikube_domain::{Age, Quantity};

/// How a cell should be coloured, by meaning. The table view maps these to theme tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tone {
    /// No emphasis. The default.
    #[default]
    Neutral,
    /// Healthy: `Running`, `Ready`, `Bound`.
    Ok,
    /// In transition or degraded: `Pending`, `Terminating`, `NotReady`.
    Warn,
    /// Failed: `CrashLoopBackOff`, `Error`, `Failed`.
    Error,
}

/// The typed value a cell sorts by.
///
/// [`CellSort::Text`] sorts by the cell's display text, ASCII-case-insensitively. Every other
/// variant carries its own value. Across variants the order is a fixed ladder, so a column that
/// mixes kinds (a CRD string column holding `3` and `1Gi`) still sorts deterministically:
/// plain numbers ([`CellSort::Int`] and [`CellSort::Float`], compared by value), then
/// quantities, then ages, then text, then times, and a cell without a value
/// ([`CellSort::None`]) after everything, so blanks sink to the bottom when ascending (the same
/// order as the store's own sort).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CellSort {
    /// No value: the cell is blank (a missing field, a metric not reported yet).
    #[default]
    None,
    /// An integer count: restarts, replicas, a port number.
    Int(i64),
    /// A real number or a ratio: ready containers over total.
    Float(f64),
    /// A resource quantity, ordered by exact value (`500Mi` before `2Gi`).
    Quantity(Quantity),
    /// A span since something happened (`Age`, `Last Schedule`), ordered by length: youngest
    /// first when ascending.
    Age(Age),
    /// A point in time, ordered chronologically.
    Time(Timestamp),
    /// The cell's display text.
    Text,
}

impl CellSort {
    fn rank(&self) -> u8 {
        match self {
            CellSort::Int(_) | CellSort::Float(_) => 0,
            CellSort::Quantity(_) => 1,
            CellSort::Age(_) => 2,
            CellSort::Text => 3,
            CellSort::Time(_) => 4,
            CellSort::None => 5,
        }
    }
}

/// The value of a ready cell.
#[derive(Debug, Clone, PartialEq)]
pub struct CellValue<'a> {
    /// What the cell shows. Empty for a blank cell.
    pub text: Cow<'a, str>,
    /// What the cell sorts by.
    pub sort: CellSort,
    /// How the cell should be coloured.
    pub tone: Tone,
}

/// One table cell for one object and one column.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell<'a> {
    /// A value, possibly blank.
    Value(CellValue<'a>),
    /// A value the column promises but nothing has supplied yet: CPU and memory before a metrics
    /// source is registered or has reported (E13). It shows nothing, never `0`, and sorts last.
    Pending,
}

impl<'a> Cell<'a> {
    /// A blank cell (the field is absent).
    pub const fn empty() -> Self {
        Cell::Value(CellValue {
            text: Cow::Borrowed(""),
            sort: CellSort::None,
            tone: Tone::Neutral,
        })
    }

    /// A text cell sorted by its text. An empty string is a blank cell.
    pub fn text(text: impl Into<Cow<'a, str>>) -> Self {
        let text = text.into();
        let sort = if text.is_empty() {
            CellSort::None
        } else {
            CellSort::Text
        };
        Cell::Value(CellValue {
            text,
            sort,
            tone: Tone::Neutral,
        })
    }

    /// An integer cell.
    pub fn int(n: i64) -> Self {
        Self::shown(n.to_string(), CellSort::Int(n))
    }

    /// A cell whose text is `text` but which sorts by the real number `n`.
    pub fn float(text: impl Into<Cow<'a, str>>, n: f64) -> Self {
        Self::shown(text, CellSort::Float(n))
    }

    /// A cell showing `text` (as the API spelled it, for example `10Gi`) that sorts by `value`.
    pub fn quantity(text: impl Into<Cow<'a, str>>, value: Quantity) -> Self {
        Self::shown(text, CellSort::Quantity(value))
    }

    /// An age cell: kubectl text (`3d5h`), sorted by length.
    pub fn age(age: Age) -> Self {
        Self::shown(age.to_string(), CellSort::Age(age))
    }

    /// A cell showing `text` that sorts chronologically by `at`.
    pub fn time(text: impl Into<Cow<'a, str>>, at: Timestamp) -> Self {
        Self::shown(text, CellSort::Time(at))
    }

    /// A cell showing `text` with an explicit sort key.
    pub fn shown(text: impl Into<Cow<'a, str>>, sort: CellSort) -> Self {
        Cell::Value(CellValue {
            text: text.into(),
            sort,
            tone: Tone::Neutral,
        })
    }

    /// The same cell with `tone`. A [`Cell::Pending`] stays as it is.
    #[must_use]
    pub fn with_tone(mut self, tone: Tone) -> Self {
        if let Cell::Value(v) = &mut self {
            v.tone = tone;
        }
        self
    }

    /// The text to show: empty for a blank or pending cell.
    pub fn display(&self) -> &str {
        match self {
            Cell::Value(v) => &v.text,
            Cell::Pending => "",
        }
    }

    /// The sort key; [`CellSort::None`] for a pending cell.
    pub fn sort(&self) -> CellSort {
        match self {
            Cell::Value(v) => v.sort,
            Cell::Pending => CellSort::None,
        }
    }

    /// The tone; [`Tone::Neutral`] for a pending cell.
    pub fn tone(&self) -> Tone {
        match self {
            Cell::Value(v) => v.tone,
            Cell::Pending => Tone::Neutral,
        }
    }

    /// Whether the cell is [`Cell::Pending`].
    pub fn is_pending(&self) -> bool {
        matches!(self, Cell::Pending)
    }

    /// Whether the cell shows nothing (blank or pending).
    pub fn is_blank(&self) -> bool {
        self.sort() == CellSort::None && self.display().is_empty()
    }

    /// Orders two cells of the same column, ascending, by their sort keys. Typed values compare
    /// by value; [`CellSort::Text`] cells compare by display text, ASCII-case-insensitively;
    /// blank and pending cells come last.
    pub fn compare(&self, other: &Cell<'_>) -> Ordering {
        use CellSort::{Age, Float, Int, Quantity, Text, Time};
        let (a, b) = (self.sort(), other.sort());
        match (a, b) {
            (Int(x), Int(y)) => x.cmp(&y),
            (Float(x), Float(y)) => x.total_cmp(&y),
            (Int(x), Float(y)) => cmp_int_float(x, y),
            (Float(x), Int(y)) => cmp_int_float(y, x).reverse(),
            (Quantity(x), Quantity(y)) => x.cmp(&y),
            (Age(x), Age(y)) => x.cmp(&y),
            (Time(x), Time(y)) => x.cmp(&y),
            (Text, Text) => cmp_ignore_case(self.display(), other.display()),
            _ => a.rank().cmp(&b.rank()),
        }
    }
}

/// Orders an integer against a float exactly: by the float approximation first, and by the
/// integers themselves when that ties, so two integers above 2^53 keep their order relative to
/// any float (a lossy cast alone would make the comparison intransitive).
fn cmp_int_float(x: i64, y: f64) -> Ordering {
    (x as f64).total_cmp(&y).then_with(|| x.cmp(&(y as i64)))
}

/// ASCII-case-insensitive byte order without allocating.
fn cmp_ignore_case(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}
