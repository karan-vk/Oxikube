//! The Namespace panel's allocation section (E13-S11): the pods of the namespace summed, against
//! its `ResourceQuota`s and with its live usage.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | when to sum again: looking at the store, summing, the usage the panel polls for |
//! | `state` | [`AllocState`]: what the panel holds, following the store, the notes when something is not available |
//! | `sums` | the sums, computed on the background executor with the E13-S02 functions |
//! | `quotas` | the quotas as the server counts them (`used` against `hard`) |
//!
//! # Where the data comes from
//!
//! The panel holds a [`CountsLease`] on the Pod and ResourceQuota feeds of its namespace (through
//! the store, so the watch budget decides and a feed another view has open is shared) and reads
//! the namespace's objects from the caches once a second. It never lists pods itself. A look is
//! one revision read per feed (`ResourceStore::namespace_revision`): when neither moved nothing is
//! cloned or compared. When one did, the objects are compared with the ones summed last (the very
//! same `Arc`s, by address, the old ones kept alive so no address can be reused); only when a pod
//! or quota changed is the sum redone on the background executor (about 1 us a pod: 2 ms for
//! 2 000 pods, `cargo bench -p oxikube_app --bench utilisation`) and the result replaces the
//! previous one, so a frame only draws ready-made rows.
//!
//! # When it is not available
//!
//! Each reason is said: the pods feed still listing, refused (no access, the watch budget),
//! failed, or reduced to metadata (no specs, so no totals). Quotas that cannot be read leave the
//! totals in place and say so.

mod quotas;
mod state;
mod sums;

use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext as _, Context};
use oxikube_app::allocation::UsageState;
use oxikube_app::store::StoreObject;
use oxikube_domain::ids::Gvk;
use oxikube_domain::utilisation::GpuVendors;
use oxikube_settings::MetricsSettings;

#[cfg(test)]
pub(super) use self::quotas::line_text;
pub(super) use self::quotas::quotas_block;
pub(super) use self::state::AllocState;
use super::panel::NamespacePanel;
use crate::kinds::allocation::{UsageSubject, UsageTarget};

/// How often the panel looks at the namespace's objects while it has nothing to show.
pub(super) const TICK_WAITING: Duration = Duration::from_millis(250);
/// How often it looks once it does.
pub(super) const TICK: Duration = Duration::from_secs(1);

impl NamespacePanel {
    /// Brings the allocation up to date with the store. Cheap when nothing moved (the tick calls
    /// it once a second): two revision reads. Returns whether anything visible changed.
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
        let Some(store) = self.alloc.store().cloned() else {
            return changed;
        };
        let pods_gvk = Gvk::new("", "v1", "Pod");
        let quotas_gvk = Gvk::new("", "v1", "ResourceQuota");
        let Some(pods_revision) = store.namespace_revision(&pods_gvk, &namespace) else {
            changed |= self.alloc.explain_missing_pods();
            return changed;
        };
        let quotas_revision = store.namespace_revision(&quotas_gvk, &namespace);
        changed |= self.alloc.explain_quotas(quotas_revision.is_some());
        if self.alloc.is_current((pods_revision, quotas_revision)) {
            return changed;
        }
        let (Some(pods), quotas) = (
            store.namespace_objects(&pods_gvk, &namespace),
            store.namespace_objects(&quotas_gvk, &namespace),
        ) else {
            return changed;
        };
        if self.alloc.followed_objects(&pods, &quotas) {
            self.alloc.note_seen((pods_revision, quotas_revision));
            return changed;
        }
        if !sums::totals_possible(&pods) {
            self.alloc
                .cannot_total(pods, quotas, (pods_revision, quotas_revision));
            return true;
        }
        let gpus = self
            .alloc
            .gpus_or_else(|| MetricsSettings::gpu_table(cx).for_cluster(&cluster).clone());
        self.sum(
            namespace,
            pods,
            quotas,
            gpus,
            (pods_revision, quotas_revision),
            cx,
        );
        changed
    }

    /// Sums on the background executor and installs the result when it lands.
    fn sum(
        &mut self,
        namespace: Arc<str>,
        pods: Vec<Arc<StoreObject>>,
        quotas: Option<Vec<Arc<StoreObject>>>,
        gpus: GpuVendors,
        revisions: (u64, Option<u64>),
        cx: &mut Context<Self>,
    ) {
        self.alloc.follow_objects(&pods, &quotas, revisions);
        let task = cx.spawn(async move |this, cx| {
            let sums = cx
                .background_spawn(async move {
                    sums::compute(&namespace, &pods, quotas.as_deref().unwrap_or(&[]), &gpus)
                })
                .await;
            this.update(cx, |panel, cx| {
                panel.alloc.install(sums);
                cx.notify();
            })
            .ok();
        });
        self.alloc.hold_compute(task);
    }

    /// A new answer from the metrics port.
    pub(super) fn usage_arrived(&mut self, usage: UsageState, cx: &mut Context<Self>) {
        self.alloc.set_usage(usage);
        cx.notify();
    }

    /// What the usage poll asks, and of whom.
    pub(super) fn usage_target(&self, cx: &App) -> Option<UsageTarget> {
        let detail = self.detail.upgrade()?;
        let view = detail.read(cx);
        let namespace = Arc::from(&*view.target().name);
        UsageTarget::of(view, UsageSubject::Namespace(namespace))
    }
}
