//! The tab as plain data: [`Stage`], the preview it showed and what the person may look at.

use oxikube_app::UpgradePreview;
use oxikube_app::guard::ConfirmationRequest;
use oxikube_domain::helm::ReleaseRef;

use super::form::UpgradeForm;

/// Where the upgrade is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// Looking for `helm` and reading the release.
    Loading,
    /// No usable `helm` binary: the tab explains what to do and offers nothing else. The
    /// release's current values are still readable (natively) but nothing can change.
    NoHelm,
    /// No release was named: the cluster's releases are listed to pick from.
    Choosing,
    /// The form and the values are being edited.
    Editing,
    /// The dry run is running.
    Previewing,
    /// Waiting for the guard's confirmation request.
    Asking,
    /// The guard asks: the tab shows its text with Upgrade and Cancel.
    Confirming(ConfirmationRequest),
    /// `helm upgrade` is running; its output streams into the log.
    Upgrading,
    /// The release was upgraded; the handler's message.
    Upgraded(String),
    /// The upgrade failed or was refused; why.
    Failed(String),
}

impl Stage {
    /// Whether the form and the editor take input (nothing is running or asking).
    pub fn is_editable(&self) -> bool {
        matches!(self, Self::Editing | Self::Failed(_))
    }
}

/// What the tab tells its owner (it is an [`EventEmitter`](gpui::EventEmitter) of it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelmUpgradeEvent {
    /// `helm upgrade` succeeded: the release. The releases view (E16-S05) refreshes from the
    /// release Secrets' watch.
    Upgraded(ReleaseRef),
}

/// What the right column shows while editing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    /// The edited values against the release's current values.
    #[default]
    ValuesDiff,
    /// The chart's default values at the current version against the target version.
    ChartDefaults,
    /// The preview: objects added, removed and changed, and the rendered manifest.
    Preview,
}

/// A preview and what it was made from, so an edit since makes it stale.
#[derive(Debug, Clone)]
pub struct Shown {
    /// The preview.
    pub preview: UpgradePreview,
    /// The form it was made from.
    pub form: UpgradeForm,
    /// The editor version of the values it was made from.
    pub values_version: u64,
}

impl Shown {
    /// Whether the form and values are still those previewed.
    pub fn is_fresh(&self, form: &UpgradeForm, values_version: u64) -> bool {
        &self.form == form && self.values_version == values_version
    }
}
