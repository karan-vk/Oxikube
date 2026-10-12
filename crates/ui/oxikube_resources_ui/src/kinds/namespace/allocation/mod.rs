//! The Namespace panel's allocation section (E13-S11): the pods of the namespace summed, against
//! its `ResourceQuota`s and with its live usage.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`AllocState`]: following the store, deciding when to sum again, the notes when something is not available |
//! | `sums` | the sums, computed on the background executor with the E13-S02 functions |
//! | `quotas` | the quotas as the server counts them (`used` against `hard`) |
//!
//! # Where the data comes from
//!
//! The panel holds a [`CountsLease`] on the Pod and ResourceQuota feeds of its namespace (through
//! the store, so the watch budget decides and a feed another view has open is shared) and reads
//! the namespace's objects from the caches once a second. It never lists pods itself. When the
//! objects are the ones it summed last (the very same `Arc`s, compared by address while the old
//! ones are kept alive so no address can be reused) nothing happens; when a pod changed, the sum
//! is redone on the background executor (about 1 us a pod: 2 ms for 2 000 pods, `cargo bench -p oxikube_app --bench utilisation`) and the
//! result replaces the previous one, so a frame only draws ready-made rows.
//!
//! # When it is not available
//!
//! Each reason is said: the pods feed still listing, refused (no access, the watch budget),
//! failed, or reduced to metadata (no specs, so no totals). Quotas that cannot be read leave the
//! totals in place and say so.

mod quotas;
mod sums;

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, Context, Task};
use oxikube_app::allocation::{AllocationView, UsageState};
use oxikube_app::store::{CountState, CountTarget, CountsLease, ResourceStore, StoreObject};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::utilisation::GpuVendors;
use oxikube_settings::MetricsSettings;

pub(super) use self::quotas::quotas_block;
use self::sums::{QuotaView, Sums};
use super::panel::NamespacePanel;
use crate::kinds::allocation::{UsageSubject, UsageTarget};

/// How often the panel looks at the namespace's objects while it has nothing to show.
pub(super) const TICK_WAITING: Duration = Duration::from_millis(250);
/// How often it looks once it does.
pub(super) const TICK: Duration = Duration::from_secs(1);

fn pods_target() -> Option<CountTarget> {
    CountTarget::core("", "pods")
}

fn quotas_target() -> Option<CountTarget> {
    CountTarget::core("", "resourcequotas")
}

/// What the sums were last made from: the objects themselves, kept alive on purpose so that
/// pointer equality cannot be fooled by a reused address, and the GPU list.
struct Followed {
    pods: Vec<Arc<StoreObject>>,
    quotas: Option<Vec<Arc<StoreObject>>>,
    gpus: GpuVendors,
}

impl Followed {
    fn is_current(
        &self,
        pods: &[Arc<StoreObject>],
        quotas: &Option<Vec<Arc<StoreObject>>>,
        gpus: &GpuVendors,
    ) -> bool {
        let same = |a: &[Arc<StoreObject>], b: &[Arc<StoreObject>]| {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| Arc::ptr_eq(a, b))
        };
        self.gpus == *gpus
            && same(&self.pods, pods)
            && match (&self.quotas, quotas) {
                (Some(a), Some(b)) => same(a, b),
                (None, None) => true,
                _ => false,
            }
    }
}

/// The allocation state of a [`NamespacePanel`].
#[derive(Default)]
pub(super) struct AllocState {
    store: Option<ResourceStore>,
    namespace: Option<Arc<str>>,
    lease: Option<CountsLease>,
    followed: Option<Followed>,
    sums: Option<Sums>,
    usage: UsageState,
    view: Option<AllocationView>,
    /// Why there is no view, or what is missing from it.
    note: Option<String>,
    /// Why the quotas could not be read, when they could not.
    quota_note: Option<String>,
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

    /// Forgets what the sums were made from, so the next look sums again (the GPU list changed).
    pub fn invalidate(&mut self) {
        self.followed = None;
    }
}

impl NamespacePanel {
    /// Brings the allocation up to date with the store. Cheap when nothing moved (the tick calls
    /// it once a second): two cache reads and a pointer comparison per pod.
    pub(super) fn refresh_allocation(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(detail) = self.detail.upgrade() else {
            return false;
        };
        let (namespace, cluster, store) = {
            let view = detail.read(cx);
            (
                Arc::<str>::from(&*view.target().name),
                view.target().cluster.clone(),
                view.store().cloned(),
            )
        };
        let mut changed = self.alloc.follow(&namespace, store);
        let Some(store) = self.alloc.store.clone() else {
            return changed;
        };
        let pods_gvk = Gvk::new("", "v1", "Pod");
        let quotas_gvk = Gvk::new("", "v1", "ResourceQuota");
        let Some(pods) = store.namespace_objects(&pods_gvk, &namespace) else {
            changed |= self.alloc.explain_missing_pods();
            return changed;
        };
        let quotas = store.namespace_objects(&quotas_gvk, &namespace);
        changed |= self.alloc.explain_quotas(quotas.is_some());
        let gpus = MetricsSettings::gpu_table(cx).for_cluster(&cluster).clone();
        if self
            .alloc
            .followed
            .as_ref()
            .is_some_and(|f| f.is_current(&pods, &quotas, &gpus))
        {
            return changed;
        }
        if !sums::totals_possible(&pods) {
            self.alloc.sums = None;
            self.alloc.view = None;
            self.alloc.note = Some(
                "The pods of this namespace are loaded without their specs (the watch budget \
                 reduced the feed), so requests and limits cannot be totalled."
                    .to_owned(),
            );
            self.alloc.followed = Some(Followed { pods, quotas, gpus });
            return true;
        }
        self.sum(namespace, pods, quotas, gpus, cx);
        changed
    }

    /// Sums on the background executor and installs the result when it lands.
    fn sum(
        &mut self,
        namespace: Arc<str>,
        pods: Vec<Arc<StoreObject>>,
        quotas: Option<Vec<Arc<StoreObject>>>,
        gpus: GpuVendors,
        cx: &mut Context<Self>,
    ) {
        self.alloc.followed = Some(Followed {
            pods: pods.clone(),
            quotas: quotas.clone(),
            gpus: gpus.clone(),
        });
        self.alloc.compute = Some(cx.spawn(async move |this, cx| {
            let sums = cx
                .background_spawn(async move {
                    sums::compute(&namespace, &pods, quotas.as_deref().unwrap_or(&[]), &gpus)
                })
                .await;
            this.update(cx, |panel, cx| {
                panel.alloc.note = None;
                panel.alloc.view = Some(sums.view(&panel.alloc.usage));
                panel.alloc.sums = Some(sums);
                cx.notify();
            })
            .ok();
        }));
    }

    /// A new answer from the metrics port.
    pub(super) fn usage_arrived(&mut self, usage: UsageState, cx: &mut Context<Self>) {
        self.alloc.usage = usage;
        if let Some(sums) = &self.alloc.sums {
            self.alloc.view = Some(sums.view(&self.alloc.usage));
        }
        cx.notify();
    }

    /// What the usage poll asks, and of whom.
    pub(super) fn usage_target(&self, cx: &App) -> Option<UsageTarget> {
        let detail = self.detail.upgrade()?;
        let view = detail.read(cx);
        let cluster: ClusterId = view.target().cluster.clone();
        let port = view.deps().sessions.get(&cluster)?.metrics()?;
        Some(UsageTarget {
            port,
            cluster,
            subject: UsageSubject::Namespace(Arc::from(&*view.target().name)),
        })
    }
}

impl AllocState {
    /// Takes the lease on the namespace's feeds (a reconnect is a new store: the lease moves with
    /// it and what was summed is dropped). Returns whether anything visible changed.
    fn follow(&mut self, namespace: &Arc<str>, store: Option<ResourceStore>) -> bool {
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
        self.followed = None;
        self.sums = None;
        self.view = None;
        self.compute = None;
        true
    }

    /// Says why the pods are not there: still listing, refused, failed.
    fn explain_missing_pods(&mut self) -> bool {
        let state = self
            .lease
            .as_ref()
            .and_then(|lease| lease.counts().into_iter().next());
        let note = match state {
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
        self.view = None;
        self.sums = None;
        self.followed = None;
        self.replace_note(Some(note))
    }

    /// Says why the quotas are not there, when they are not. They load with the pods, so a
    /// missing list is a refusal or a failure, or still listing.
    fn explain_quotas(&mut self, listed: bool) -> bool {
        let note = (!listed).then(|| {
            let state = self
                .lease
                .as_ref()
                .and_then(|lease| lease.counts().into_iter().nth(1));
            match state {
                Some(CountState::NoAccess { .. }) => {
                    "You are not allowed to list the ResourceQuotas of this namespace, so the \
                     totals have nothing to be compared with."
                        .to_owned()
                }
                Some(CountState::OverBudget { message } | CountState::Failed { message }) => {
                    format!("The ResourceQuotas of this namespace could not be read: {message}")
                }
                _ => "Reading the ResourceQuotas of this namespace...".to_owned(),
            }
        });
        let changed = note != self.quota_note;
        self.quota_note = note;
        changed
    }

    fn replace_note(&mut self, note: Option<String>) -> bool {
        let changed = note != self.note;
        self.note = note;
        changed
    }
}
