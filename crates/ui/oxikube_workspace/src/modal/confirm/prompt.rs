//! [`ConfirmPrompt`]: everything a [`ConfirmDialog`](super::ConfirmDialog) shows, as plain data.
//!
//! The prompt is a view model. It carries no confirmation token and no handle to the bus: the
//! caller that builds it keeps the token in the closures it hands the dialog, so a dialog can
//! only ever answer the one request it was opened for, and the data here can be built in a test
//! or a screenshot without a guard.

use gpui::SharedString;
use oxikube_app::PreviewChange;
use oxikube_domain::ClusterColour;
use oxikube_domain::audit::Initiator;
use oxikube_domain::safety::Risk;

/// One object a bulk request acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfirmItem {
    /// `Kind namespace/name`, as the user knows the object.
    pub label: SharedString,
    /// The blast radius of this object alone.
    pub risk: Risk,
}

impl ConfirmItem {
    /// An item labelled `label` with `risk`.
    pub fn new(label: impl Into<SharedString>, risk: Risk) -> Self {
        Self {
            label: label.into(),
            risk,
        }
    }
}

/// What the guard's server-side dry run found, for the dialog's banner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewNote {
    /// The server accepted a dry run; its words (`Dry run: namespace demo can be deleted`).
    Rehearsed(SharedString),
    /// The server could not dry-run the request; why. The mutation may still run once confirmed.
    Unavailable(SharedString),
}

/// What a confirmation dialog shows. Built with the `with_*` methods from [`ConfirmPrompt::new`].
#[derive(Clone, Debug)]
pub struct ConfirmPrompt {
    pub(super) title: SharedString,
    pub(super) summary: SharedString,
    pub(super) confirm_label: SharedString,
    pub(super) destructive: bool,
    pub(super) expected: Option<SharedString>,
    pub(super) cluster: Option<(SharedString, Option<ClusterColour>)>,
    pub(super) initiator: Initiator,
    pub(super) risk: Option<Risk>,
    pub(super) preview: Option<PreviewNote>,
    pub(super) changes: Vec<PreviewChange>,
    pub(super) omitted: usize,
    pub(super) warnings: Vec<SharedString>,
    pub(super) items: Vec<ConfirmItem>,
}

impl ConfirmPrompt {
    /// A prompt titled `title` with the one-line `summary` of what will happen, a plain
    /// "Confirm" button and no typed name.
    pub fn new(title: impl Into<SharedString>, summary: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            confirm_label: "Confirm".into(),
            destructive: false,
            expected: None,
            cluster: None,
            initiator: Initiator::Ui,
            risk: None,
            preview: None,
            changes: Vec::new(),
            omitted: 0,
            warnings: Vec::new(),
            items: Vec::new(),
        }
    }

    /// The confirm button's label.
    #[must_use]
    pub fn with_confirm_label(mut self, label: impl Into<SharedString>) -> Self {
        self.confirm_label = label.into();
        self
    }

    /// Draws the confirm button as a dangerous action.
    #[must_use]
    pub fn with_destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    /// Makes the user type `expected` (case-sensitive, no trimming, the same comparison the guard
    /// makes) before the dialog confirms.
    #[must_use]
    pub fn with_typed_name(mut self, expected: impl Into<SharedString>) -> Self {
        self.expected = Some(expected.into());
        self
    }

    /// The cluster the request acts on: its name and its badge colour, if it has one.
    #[must_use]
    pub fn with_cluster(
        mut self,
        name: impl Into<SharedString>,
        colour: Option<ClusterColour>,
    ) -> Self {
        self.cluster = Some((name.into(), colour));
        self
    }

    /// Who asked.
    #[must_use]
    pub fn with_initiator(mut self, initiator: Initiator) -> Self {
        self.initiator = initiator;
        self
    }

    /// The command's blast radius.
    #[must_use]
    pub fn with_risk(mut self, risk: Option<Risk>) -> Self {
        self.risk = risk;
        self
    }

    /// The guard's dry-run result.
    #[must_use]
    pub fn with_preview(mut self, preview: Option<PreviewNote>) -> Self {
        self.preview = preview;
        self
    }

    /// What the guard's dry run says the request would write (E19-S07): each object next to its
    /// live version (Secret values already fingerprints), drawn as a diff, and how many more
    /// objects it wrote than `changes` holds.
    #[must_use]
    pub fn with_changes(mut self, changes: Vec<PreviewChange>, omitted: usize) -> Self {
        self.changes = changes;
        self.omitted = omitted;
        self
    }

    /// The API server's warnings about the dry run (deprecations, unknown fields), shown next to
    /// the diff.
    #[must_use]
    pub fn with_warnings(mut self, warnings: Vec<SharedString>) -> Self {
        self.warnings = warnings;
        self
    }

    /// The objects of a bulk request, listed (virtualised) under the summary.
    #[must_use]
    pub fn with_items(mut self, items: Vec<ConfirmItem>) -> Self {
        self.items = items;
        self
    }

    /// Whether the items differ in risk, so each row should say its own.
    pub(super) fn items_differ_in_risk(&self) -> bool {
        self.items
            .split_first()
            .is_some_and(|(first, rest)| rest.iter().any(|item| item.risk != first.risk))
    }
}

/// Whether `typed` satisfies a type-the-name prompt for `expected`: exact, case-sensitive, no
/// trimming. The guard compares the same way (`ConfirmationError::NameMismatch`), so a dialog
/// that enables Confirm is a dialog the guard accepts, and the other way round. A plain string
/// compare: it runs on every keystroke.
pub fn typed_matches(typed: &str, expected: &str) -> bool {
    typed == expected
}
