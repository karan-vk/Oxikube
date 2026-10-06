//! [`Row`]: the Overview as one flat list, so a long label or annotation list is virtualised
//! like any other list (only the rows on screen are built).

use super::DetailModel;

/// One row of the Overview body. Rows name their data by index into the [`DetailModel`], so a
/// row is cheap and the model stays the one place the text lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    /// A section heading with its count (`Labels`, 3).
    Section(Section, usize),
    /// A line saying a section is empty.
    Empty(Section),
    /// Label `n`.
    Label(usize),
    /// Annotation `n`.
    Annotation(usize),
    /// Owner reference `n`.
    Owner(usize),
    /// Finalizer `n`.
    Finalizer(usize),
    /// The column headings of the conditions table.
    ConditionHead,
    /// Condition `n`.
    Condition(usize),
    /// Status line `n`.
    Status(usize),
    /// A note that the summary was cut at its line limit.
    StatusTruncated,
    /// Secret key `n` (a name, never a value).
    SecretKey(usize),
    /// The object's `spec` and `status` are still being read.
    Loading,
}

/// The sections of the Overview, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Labels.
    Labels,
    /// Annotations.
    Annotations,
    /// Owner references.
    Owners,
    /// Finalizers.
    Finalizers,
    /// Conditions.
    Conditions,
    /// The `status` summary.
    Status,
    /// A Secret's key names.
    Keys,
}

impl Section {
    /// The heading text.
    pub fn title(self) -> &'static str {
        match self {
            Section::Labels => "Labels",
            Section::Annotations => "Annotations",
            Section::Owners => "Owned by",
            Section::Finalizers => "Finalizers",
            Section::Conditions => "Conditions",
            Section::Status => "Status",
            Section::Keys => "Data keys",
        }
    }

    /// What an empty section says.
    pub fn empty_text(self) -> &'static str {
        match self {
            Section::Labels => "No labels",
            Section::Annotations => "No annotations",
            Section::Owners => "No owner",
            Section::Finalizers => "No finalizers",
            Section::Conditions => "No conditions",
            Section::Status => "No status",
            Section::Keys => "No keys",
        }
    }
}

pub(super) fn flatten(model: &DetailModel) -> Vec<Row> {
    let mut rows = Vec::with_capacity(
        model.labels.len() + model.annotations.len() + model.conditions.len() + 16,
    );
    section(&mut rows, Section::Owners, model.owners.len(), Row::Owner);
    section(&mut rows, Section::Labels, model.labels.len(), Row::Label);
    section(
        &mut rows,
        Section::Annotations,
        model.annotations.len(),
        Row::Annotation,
    );
    section(
        &mut rows,
        Section::Finalizers,
        model.finalizers.len(),
        Row::Finalizer,
    );
    if let Some(keys) = &model.secret_keys {
        if !model.complete {
            // The key names come from the full read: until it lands (or while it has failed) the
            // Secret is not known to have no keys.
            rows.push(Row::Section(Section::Keys, 0));
            rows.push(Row::Loading);
            return rows;
        }
        section(&mut rows, Section::Keys, keys.len(), Row::SecretKey);
        return rows;
    }
    rows.push(Row::Section(Section::Conditions, model.conditions.len()));
    if !model.complete {
        rows.push(Row::Loading);
        return rows;
    }
    if model.conditions.is_empty() {
        rows.push(Row::Empty(Section::Conditions));
    } else {
        rows.push(Row::ConditionHead);
        rows.extend((0..model.conditions.len()).map(Row::Condition));
    }
    let lines = model.status.lines.len();
    rows.push(Row::Section(Section::Status, lines));
    if lines == 0 {
        rows.push(Row::Empty(Section::Status));
    } else {
        rows.extend((0..lines).map(Row::Status));
        if model.status.truncated {
            rows.push(Row::StatusTruncated);
        }
    }
    rows
}

/// Pushes a section. Owners and finalizers are left out when there are none (most objects have
/// neither); the other sections say they are empty.
fn section(rows: &mut Vec<Row>, section: Section, len: usize, row: fn(usize) -> Row) {
    if len == 0 && matches!(section, Section::Owners | Section::Finalizers) {
        return;
    }
    rows.push(Row::Section(section, len));
    if len == 0 {
        rows.push(Row::Empty(section));
    } else {
        rows.extend((0..len).map(row));
    }
}
