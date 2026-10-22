//! [`ReleaseDetail`]: the selected release's tabs and what each has read.
//!
//! Only the shown tab reads (the others stay [`Load::Idle`] until opened), every read runs off the
//! UI thread (`spawn_kube`) and decodes nothing on it, and a read in flight is dropped (and so
//! cancelled) with the detail or when a newer one replaces it. A tab that has read stays read
//! until the release moves to a new revision or the reveal changes.

use std::collections::HashMap;

use gpui::{AppContext as _, Context, Entity, Task, UniformListScrollHandle};
use oxikube_domain::helm::{Release, Revision};
use oxikube_ui::code_view::{CodeView, Look};

use super::backend::{ObjectRow, ObjectsLease};
use super::change::Preview;
use super::model::{Load, Tab, ValuesMode};
use super::view::HelmReleasesView;

/// A tab that shows text in a [`CodeView`].
pub struct TextPane {
    /// How far the read is.
    pub load: Load,
    /// The text on screen.
    pub code: Entity<CodeView>,
    /// The read found nothing to show (no user values, no notes).
    pub empty: bool,
}

impl TextPane {
    fn new(look: Look, cx: &mut Context<HelmReleasesView>) -> Self {
        Self {
            load: Load::Idle,
            code: cx.new(|cx| CodeView::new(look, cx)),
            empty: false,
        }
    }
}

/// What the Values tab knows beyond its text.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ValuesFacts {
    /// How many values are (or, once revealed, would be) masked.
    pub sensitive: usize,
    /// The text shows them in clear.
    pub revealed: bool,
}

/// The Resources tab.
pub struct ResourcesPane {
    /// How far the read is (the manifest, its mapping and the feeds opening).
    pub load: Load,
    /// The release's objects with their state, in manifest order.
    pub objects: Vec<ObjectRow>,
    /// The feeds held while the tab shows.
    pub(super) lease: Option<Box<dyn ObjectsLease>>,
    /// The list's scroll position.
    pub scroll: UniformListScrollHandle,
    /// The once-a-second refresh of the objects' state while the tab is shown.
    pub(super) tick: Option<Task<()>>,
}

/// The History tab.
pub struct HistoryPane {
    /// How far the read is.
    pub load: Load,
    /// Every revision, newest first.
    pub revisions: Vec<Revision>,
    /// The list's scroll position.
    pub scroll: UniformListScrollHandle,
    /// The revision picked for rollback (E16-S06).
    pub selected: Option<u32>,
    /// What rolling back to it would change.
    pub preview: Preview,
    pub(super) preview_task: Option<Task<()>>,
}

/// The detail of the selected release. See the module docs.
pub struct ReleaseDetail {
    /// The row it was opened from: the latest revision's summary.
    pub release: Release,
    /// The tab shown.
    pub tab: Tab,
    /// User or computed values.
    pub mode: ValuesMode,
    /// The Resources tab.
    pub resources: ResourcesPane,
    /// The Values tab.
    pub values: TextPane,
    /// What the Values tab says about masking.
    pub values_facts: ValuesFacts,
    /// The Manifest tab.
    pub manifest: TextPane,
    /// Whether the manifest's Secret data is masked.
    pub manifest_masked: bool,
    /// The Notes tab.
    pub notes: TextPane,
    /// The links in the notes that may be opened.
    pub links: Vec<String>,
    /// The History tab.
    pub history: HistoryPane,
    /// The uninstall in the header keeps the release history (`--keep-history`).
    pub keep_history: bool,
    /// Counts the changes that make a read in flight stale (a new revision, a new mode, a reveal).
    pub(super) generation: u64,
    /// The reads in flight, one per tab. Dropped with the detail.
    pub(super) tasks: HashMap<Tab, Task<()>>,
}

impl ReleaseDetail {
    pub(super) fn new(release: Release, cx: &mut Context<HelmReleasesView>) -> Self {
        Self {
            release,
            tab: Tab::Resources,
            mode: ValuesMode::User,
            resources: ResourcesPane {
                load: Load::Idle,
                objects: Vec::new(),
                lease: None,
                scroll: UniformListScrollHandle::new(),
                tick: None,
            },
            values: TextPane::new(Look::YAML, cx),
            values_facts: ValuesFacts::default(),
            manifest: TextPane::new(Look::YAML, cx),
            manifest_masked: true,
            notes: TextPane::new(Look::TEXT, cx),
            links: Vec::new(),
            history: HistoryPane {
                load: Load::Idle,
                revisions: Vec::new(),
                scroll: UniformListScrollHandle::new(),
                selected: None,
                preview: Preview::None,
                preview_task: None,
            },
            keep_history: false,
            generation: 0,
            tasks: HashMap::new(),
        }
    }

    /// Forgets what every tab read and cancels the reads in flight (the release moved to a new
    /// revision, or what the tabs show changed).
    pub(super) fn invalidate(&mut self) {
        self.generation += 1;
        self.tasks.clear();
        self.resources.lease = None;
        self.resources.tick = None;
        self.resources.load = Load::Idle;
        self.values.load = Load::Idle;
        self.manifest.load = Load::Idle;
        self.notes.load = Load::Idle;
        self.history.load = Load::Idle;
        self.history.selected = None;
        self.history.preview = Preview::None;
        self.history.preview_task = None;
    }

    /// Drops what `tabs` read and every read still in flight, so no tab is left waiting for a
    /// result the new generation would discard. The caller starts the shown tab again.
    pub(super) fn restart(&mut self, tabs: &[Tab]) {
        self.generation += 1;
        for tab in Tab::ALL {
            if tabs.contains(&tab) || matches!(self.load(tab), Load::Loading) {
                self.tasks.remove(&tab);
                if tab == Tab::Resources {
                    self.resources.tick = None;
                }
                *self.load_mut(tab) = Load::Idle;
            }
        }
    }

    /// The load state of `tab`.
    pub fn load(&self, tab: Tab) -> &Load {
        match tab {
            Tab::Resources => &self.resources.load,
            Tab::Values => &self.values.load,
            Tab::Manifest => &self.manifest.load,
            Tab::Notes => &self.notes.load,
            Tab::History => &self.history.load,
        }
    }

    pub(super) fn load_mut(&mut self, tab: Tab) -> &mut Load {
        match tab {
            Tab::Resources => &mut self.resources.load,
            Tab::Values => &mut self.values.load,
            Tab::Manifest => &mut self.manifest.load,
            Tab::Notes => &mut self.notes.load,
            Tab::History => &mut self.history.load,
        }
    }
}
