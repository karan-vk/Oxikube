//! [`Panel`]: what a Config kind's detail panel shows, as plain data.
//!
//! Each kind's builder turns the domain view-model into a [`Panel`] (sections of [`Row`]s); one
//! renderer draws any panel (`draw`). Splitting data from drawing keeps the per-kind code
//! declarative and testable without a window.

use std::sync::Arc;

use oxikube_app::columns::Tone;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::view::QuotaLevel;

/// A panel: titled sections of rows.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Panel {
    /// The sections, top to bottom.
    pub sections: Vec<Section>,
}

/// A titled group of rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Section {
    /// The heading (drawn in capitals).
    pub title: String,
    /// The rows.
    pub rows: Vec<Row>,
}

/// What a link opens: the detail of an object next to the one shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LinkTo {
    /// The linked kind.
    pub gvk: Gvk,
    /// The linked object's name.
    pub name: Arc<str>,
    /// Whether it lives in the shown object's namespace or at cluster level.
    pub scope: Scope,
}

/// One row of a section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Row {
    /// A label and a value, the value in the colour of its tone.
    Pair {
        label: String,
        value: String,
        tone: Tone,
    },
    /// A used-against-limit bar (ResourceQuota).
    Bar {
        label: String,
        /// `1500m / 4`.
        text: String,
        /// The ratio as a whole percentage, `None` when there is none to draw.
        percent: Option<u32>,
        level: QuotaLevel,
    },
    /// A label and a value that opens another object.
    Link {
        label: String,
        text: String,
        to: LinkTo,
    },
    /// Cells of a small table; the first row of a table is the `head`.
    Cells { cells: Vec<String>, head: bool },
    /// Preformatted text: a CEL expression.
    Code(String),
    /// A sub-heading inside a section: a webhook's name.
    Heading(String),
    /// Muted text: "No limits set."
    Note(String),
}

impl Row {
    /// A neutral label / value row.
    pub(crate) fn pair(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self::pair_toned(label, value, Tone::Neutral)
    }

    /// A label / value row coloured by `tone`.
    pub(crate) fn pair_toned(
        label: impl Into<String>,
        value: impl Into<String>,
        tone: Tone,
    ) -> Self {
        Row::Pair {
            label: label.into(),
            value: value.into(),
            tone,
        }
    }

    /// A pair row only when `value` is present.
    pub(crate) fn opt(label: impl Into<String>, value: Option<impl Into<String>>) -> Option<Self> {
        value.map(|v| Self::pair(label, v))
    }

    /// A table body row.
    pub(crate) fn cells(cells: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Row::Cells {
            cells: cells.into_iter().map(Into::into).collect(),
            head: false,
        }
    }

    /// A table head row.
    pub(crate) fn head(cells: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Row::Cells {
            cells: cells.into_iter().map(Into::into).collect(),
            head: true,
        }
    }

    /// The row's text, for tests that look for a value anywhere in a panel.
    #[cfg(test)]
    pub(crate) fn text(&self) -> String {
        match self {
            Row::Pair { label, value, .. } => format!("{label} {value}"),
            Row::Bar { label, text, .. } => format!("{label} {text}"),
            Row::Link { label, text, .. } => format!("{label} {text}"),
            Row::Cells { cells, .. } => cells.join(" "),
            Row::Code(text) | Row::Heading(text) | Row::Note(text) => text.clone(),
        }
    }
}

impl Panel {
    /// An empty panel.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a section; one without rows is left out.
    #[must_use]
    pub(crate) fn section(mut self, title: impl Into<String>, rows: Vec<Row>) -> Self {
        if !rows.is_empty() {
            self.sections.push(Section {
                title: title.into(),
                rows,
            });
        }
        self
    }

    /// Whether the panel has nothing to show.
    pub(crate) fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// The first row matching `pick`, for tests.
    #[cfg(test)]
    pub(crate) fn find(&self, pick: impl Fn(&Row) -> bool) -> Option<&Row> {
        self.sections.iter().flat_map(|s| &s.rows).find(|r| pick(r))
    }

    /// Every row's text, for tests.
    #[cfg(test)]
    pub(crate) fn all_text(&self) -> String {
        self.sections
            .iter()
            .flat_map(|s| std::iter::once(s.title.clone()).chain(s.rows.iter().map(Row::text)))
            .collect::<Vec<_>>()
            .join("\n")
    }
}
