//! What the Namespace panel holds for its allocation section, and why it is missing or partial.

use std::sync::Arc;

use gpui::Task;
use oxikube_app::allocation::{AllocationView, UsageState};
use oxikube_app::store::{CountState, CountTarget, CountsLease, ResourceStore, StoreObject};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::utilisation::GpuVendors;

use super::sums::{QuotaView, Sums};

/// The feed revisions a look found: pods, and quotas when their feed is listed.
pub(super) type Revisions = (u64, Option<u64>);

fn pods_target() -> Option<CountTarget> {
    CountTarget::core("", "pods")
}

fn quotas_target() -> Option<CountTarget> {
    CountTarget::core("", "resourcequotas")
}

/// What the sums were last made from: the objects themselves, kept alive on purpose so that
/// pointer equality cannot be fooled by a reused address.
struct Followed {
    pods: Vec<Arc<StoreObject>>,
    quotas: Option<Vec<Arc<StoreObject>>>,
}

/// The allocation state of a [`NamespacePanel`](super::NamespacePanel).
#[derive(Default)]
pub(in crate::kinds::namespace) struct AllocState {
    store: Option<ResourceStore>,
    namespace: Option<Arc<str>>,
    lease: Option<CountsLease>,
    /// The objects the sums were made from.
    followed: Option<Followed>,
    /// The revisions of the feeds at the last look.
    seen: Option<Revisions>,
    /// The cluster's GPU list; `None` until needed and again after `metrics.gpu_vendors` changes.
    gpus: Option<GpuVendors>,
    sums: Option<Sums>,
    usage: UsageState,
    view: Option<AllocationView>,
    /// Why there is no view, or what is missing from it.
    note: Option<String>,
    /// Why the quotas could not be read, when they could not.
    quota_note: Option<String>,
    /// The running background sum; dropping it cancels it.
    compute: Option<Task<()>>,
}

impl AllocState {
    /// The section as last built.
    pub fn view(&self) -> Option<&AllocationView> {
        self.view.as_ref()
    }

    /// Why the section is missing or incomplete.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// Why the quotas are missing.
    pub fn quota_note(&self) -> Option<&str> {
        self.quota_note.as_deref()
    }

    /// The quotas to list.
    pub fn quotas(&self) -> &[QuotaView] {
        self.sums.as_ref().map_or(&[], |s| &s.quotas)
    }

    pub(super) fn store(&self) -> Option<&ResourceStore> {
        self.store.as_ref()
    }

    /// Forgets what the sums were made from, so the next look sums again (the GPU list changed).
    pub fn invalidate(&mut self) {
        self.followed = None;
        self.seen = None;
        self.gpus = None;
    }

    /// Whether the feeds are at the revisions the sums were made at.
    pub(super) fn is_current(&self, revisions: Revisions) -> bool {
        self.followed.is_some() && self.seen == Some(revisions)
    }

    /// Whether `pods` and `quotas` are the very objects the sums were made from.
    pub(super) fn followed_objects(
        &self,
        pods: &[Arc<StoreObject>],
        quotas: &Option<Vec<Arc<StoreObject>>>,
    ) -> bool {
        let same = |a: &[Arc<StoreObject>], b: &[Arc<StoreObject>]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
        };
        self.followed.as_ref().is_some_and(|f| {
            same(&f.pods, pods)
                && match (&f.quotas, quotas) {
                    (Some(a), Some(b)) => same(a, b),
                    (None, None) => true,
                    _ => false,
                }
        })
    }

    pub(super) fn note_seen(&mut self, revisions: Revisions) {
        self.seen = Some(revisions);
    }

    /// Remembers the objects (and revisions) a sum is about to be made from.
    pub(super) fn follow_objects(
        &mut self,
        pods: &[Arc<StoreObject>],
        quotas: &Option<Vec<Arc<StoreObject>>>,
        revisions: Revisions,
    ) {
        self.followed = Some(Followed {
            pods: pods.to_vec(),
            quotas: quotas.clone(),
        });
        self.seen = Some(revisions);
    }

    /// The pods are loaded without their specs: there is nothing to total, and a sum already
    /// running is of the wrong thing.
    pub(super) fn cannot_total(
        &mut self,
        pods: Vec<Arc<StoreObject>>,
        quotas: Option<Vec<Arc<StoreObject>>>,
        revisions: Revisions,
    ) {
        self.compute = None;
        self.sums = None;
        self.view = None;
        self.note = Some(
            "The pods of this namespace are loaded without their specs (the watch budget \
             reduced the feed), so requests and limits cannot be totalled."
                .to_owned(),
        );
        self.followed = Some(Followed { pods, quotas });
        self.seen = Some(revisions);
    }

    pub(super) fn gpus_or_else(&mut self, resolve: impl FnOnce() -> GpuVendors) -> GpuVendors {
        self.gpus.get_or_insert_with(resolve).clone()
    }

    pub(super) fn hold_compute(&mut self, task: Task<()>) {
        self.compute = Some(task);
    }

    /// Installs a finished sum.
    pub(super) fn install(&mut self, sums: Sums) {
        self.note = None;
        self.view = Some(sums.view(&self.usage));
        self.sums = Some(sums);
    }

    /// Keeps the new reading and redraws the section with it.
    pub(super) fn set_usage(&mut self, usage: UsageState) {
        self.usage = usage;
        if let Some(sums) = &self.sums {
            self.view = Some(sums.view(&self.usage));
        }
    }

    /// Takes the lease on the namespace's feeds (a reconnect is a new store: the lease moves with
    /// it and what was summed is dropped). Returns whether anything visible changed.
    pub(super) fn follow(&mut self, namespace: &Arc<str>, store: Option<ResourceStore>) -> bool {
        let same_store = match (&self.store, &store) {
            (Some(a), Some(b)) => a.is_same(b),
            (None, None) => true,
            _ => false,
        };
        if same_store && self.namespace.as_ref() == Some(namespace) {
            return false;
        }
        self.lease = store.as_ref().map(|store| {
            let targets = [pods_target(), quotas_target()]
                .into_iter()
                .flatten()
                .collect();
            store.lease_counts(targets, &NamespaceSelection::from_names([&**namespace]))
        });
        self.store = store;
        self.namespace = Some(namespace.clone());
        self.invalidate();
        self.sums = None;
        self.view = None;
        self.compute = None;
        // The old namespace's reading is not this one's.
        self.usage = UsageState::Pending;
        true
    }

    /// The state of the feed of `kind` (`Pod`, `ResourceQuota`) the lease holds.
    fn state_of(&self, kind: &str) -> Option<CountState> {
        let lease = self.lease.as_ref()?;
        lease
            .targets()
            .iter()
            .zip(lease.counts())
            .find(|(target, _)| &*target.gvk.kind == kind)
            .map(|(_, state)| state)
    }

    /// Says why the pods are not there: still listing, refused, failed. A sum in flight is of
    /// pods that are gone, so it is dropped.
    pub(super) fn explain_missing_pods(&mut self) -> bool {
        let note = match self.state_of("Pod") {
            Some(CountState::NoAccess { .. }) => {
                "You are not allowed to list the pods of this namespace.".to_owned()
            }
            Some(CountState::OverBudget { message }) => {
                format!("The pods of this namespace are not watched: {message}")
            }
            Some(CountState::Failed { message }) => {
                format!("The pods of this namespace could not be listed: {message}")
            }
            _ => "Reading the pods of this namespace...".to_owned(),
        };
        self.compute = None;
        self.view = None;
        self.sums = None;
        self.followed = None;
        self.seen = None;
        let changed = self.note.as_deref() != Some(&note);
        self.note = Some(note);
        changed
    }

    /// Says why the quotas are not there, when they are not. They load with the pods, so a
    /// missing list is a refusal or a failure, or still listing.
    pub(super) fn explain_quotas(&mut self, listed: bool) -> bool {
        let note = (!listed).then(|| match self.state_of("ResourceQuota") {
            Some(CountState::NoAccess { .. }) => {
                "You are not allowed to list the ResourceQuotas of this namespace, so the \
                 totals have nothing to be compared with."
                    .to_owned()
            }
            Some(CountState::OverBudget { message } | CountState::Failed { message }) => {
                format!("The ResourceQuotas of this namespace could not be read: {message}")
            }
            _ => "Reading the ResourceQuotas of this namespace...".to_owned(),
        });
        let changed = note != self.quota_note;
        self.quota_note = note;
        changed
    }
}
