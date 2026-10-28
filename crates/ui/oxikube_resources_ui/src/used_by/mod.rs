//! "Used by" (E12-S18): what uses a ServiceAccount, ConfigMap, Secret, PersistentVolumeClaim or
//! PriorityClass, answered from the Pods' specs.
//!
//! The index and the owner walk are the app's ([`oxikube_app::references`]); this module is the
//! UI's side of them:
//!
//! | Item | Holds |
//! |---|---|
//! | [`reference_feed`] | the cluster's shared [`ReferenceFeed`] for a view that needs the references: the detail's "Used by" tab and the claims table's Pods column |
//! | [`row_actions`] | the "Used by" row action (`resource::ShowUsedBy`) for the five kinds |
//!
//! The tab itself is [`DetailTab::UsedBy`](crate::detail::DetailTab::UsedBy), drawn by the
//! detail view (`detail::used_by`).
//!
//! # How a user gets here
//!
//! Open one of the five kinds from the sidebar (Config: ConfigMaps, Secrets and Priority Classes;
//! Access Control: Service Accounts; Storage: Persistent Volume Claims) and press `u` on a row
//! (k9s's used-by), pick "Used By" in the row's menu or the command palette ("Show Used By"), or
//! open the row and choose its **Used by** tab (the fifth, key `5`). Each workload listed (a
//! Deployment, a StatefulSet, a CronJob, or a lone Pod) opens its own detail with a click.

use std::sync::Arc;

use gpui::App;
use oxikube_app::actions::KindFilter;
use oxikube_app::{ColumnsContext, KindMeta, ReferenceFeed, ReferenceHub, RowActionSpec};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Scope;
use oxikube_domain::view::RefKind;

use crate::kinds::{KindSpec, KindSpecs};

/// Where "Used By" sits in a row's menu: after "View YAML" and "Describe".
pub const USED_BY_ORDER: u16 = 140;

/// The shared reference feed of `env`'s cluster, started if nobody holds one; `None` while the
/// session is not connected (there is no store to read the Pods from yet).
///
/// The feed lives as long as a view holds the `Arc`: closing the last "Used by" tab or claims
/// table releases the Pods subscription.
pub fn reference_feed(env: &ColumnsContext) -> Option<Arc<ReferenceFeed>> {
    let store = env.store.as_ref()?;
    Some(ReferenceHub::shared().acquire(store, &env.sessions))
}

/// The row actions this module adds: "Used By" for the five referenced kinds.
pub fn row_actions() -> Vec<RowActionSpec> {
    vec![
        RowActionSpec::new(CommandId::RESOURCE_SHOW_USED_BY, |target| {
            Command::ResourceShowUsedBy {
                target: target.clone(),
            }
        })
        .label("Used By")
        .kinds(KindFilter::Matching(|kind| {
            RefKind::from_gvk(&kind.gvk).is_some()
        }))
        .order(USED_BY_ORDER),
    ]
}

/// Registers the ServiceAccount kind's spec: ConfigMaps, Secrets, claims and PriorityClasses
/// register theirs with their own modules, but ServiceAccounts have none yet (Access Control is
/// E17), so this one only names the action it offers. It has no sidebar slot: the Access Control
/// section already lists "Service Accounts", and its table and detail stay the generic ones.
pub fn init(cx: &mut App) {
    let meta = KindMeta::new(
        RefKind::ServiceAccount.gvk(),
        "serviceaccounts",
        "Service Accounts",
        Scope::Namespaced,
    )
    .with_actions([
        CommandId::RESOURCE_SHOW_USED_BY,
        // E17-S09: the rbac panel's row action; the command is the app's.
        CommandId::SERVICEACCOUNT_GENERATE_KUBECONFIG,
    ]);
    if let Err(error) = KindSpecs::register(cx, KindSpec::new(meta)) {
        tracing::error!(%error, "the ServiceAccount kind was not registered");
    }
}
