//! [`BindingChoices`] and [`role_of`] over the fakes.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use futures::FutureExt as _;
use oxikube_domain::ids::Gvk;
use oxikube_domain::rbac::{Binding, RoleKind};
use oxikube_domain::{ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::{
    ListOptions, ListPage, ResourceReader, Scale, Subresource, WatchFeed, WatchOptions,
};
use oxikube_testkit::FakeResourcePort;
use serde_json::json;

use super::role_binding;
use crate::binding_ops::{BindingChoices, role_of};
use crate::testing::{Harness, id};

fn object(value: serde_json::Value) -> Resource {
    Resource::from_json(value).expect("a valid object")
}

fn seeded() -> Harness {
    let h = Harness::with_extra(|_, _, _| Ok(()));
    h.connect("a", false);
    let port = h.resources("a");
    for value in [
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
               "metadata": {"name": "reader", "namespace": "demo"}, "rules": []}),
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "Role",
               "metadata": {"name": "other", "namespace": "elsewhere"}, "rules": []}),
        json!({"apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRole",
               "metadata": {"name": "view"}, "rules": []}),
        json!({"apiVersion": "v1", "kind": "ServiceAccount",
               "metadata": {"name": "builder", "namespace": "demo"}}),
        role_binding("view", None),
    ] {
        port.insert(object(value));
    }
    port.insert(object(json!({
        "apiVersion": "rbac.authorization.k8s.io/v1", "kind": "ClusterRoleBinding",
        "metadata": {"name": "crb"},
        "roleRef": {"apiGroup": "rbac.authorization.k8s.io", "kind": "ClusterRole", "name": "view"},
        "subjects": [{"kind": "Group", "name": "team"}, {"kind": "User", "name": "bob"}],
    })));
    h
}

#[test]
fn the_pickers_offer_what_the_cluster_has_and_the_names_bindings_already_use() {
    let h = seeded();
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("fakes answer at once")
        .expect("read");
    assert_eq!(choices.cluster_roles.len(), 1);
    assert_eq!(choices.roles.len(), 2);
    assert_eq!(choices.service_accounts.len(), 1);
    let users: Vec<&str> = choices.users.iter().map(|u| &**u).collect();
    assert_eq!(users, ["ana", "bob"]);
    let groups: Vec<&str> = choices.groups.iter().map(|u| &**u).collect();
    assert_eq!(groups, ["team"]);
    assert!(choices.notes.is_empty(), "{:?}", choices.notes);
}

#[test]
fn a_cluster_role_binding_is_offered_only_cluster_roles_and_a_role_binding_its_own_namespace() {
    let h = seeded();
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("at once")
        .expect("read");
    let names = |offered: Vec<(RoleKind, std::sync::Arc<str>)>| -> Vec<String> {
        offered
            .iter()
            .map(|(kind, name)| format!("{}/{name}", kind.as_str()))
            .collect()
    };
    assert_eq!(names(choices.offered_roles(None)), ["ClusterRole/view"]);
    assert_eq!(
        names(choices.offered_roles(Some("demo"))),
        ["Role/reader", "ClusterRole/view"]
    );
    assert_eq!(
        names(choices.offered_roles(Some("empty"))),
        ["ClusterRole/view"]
    );
}

#[test]
fn a_list_that_is_refused_is_noted_and_the_rest_still_loads() {
    let h = seeded();
    h.resources("a")
        .script()
        .list
        .push_err(OxiError::forbidden("serviceaccounts is forbidden"));
    let choices = BindingChoices::read(&h.manager, &id("a"))
        .now_or_never()
        .expect("at once")
        .expect("read");
    assert_eq!(choices.notes.len(), 1, "{:?}", choices.notes);
    assert!(choices.notes[0].contains("type the name instead"));
}

#[test]
fn the_role_a_binding_names_is_read_and_a_missing_one_is_none() {
    let h = seeded();
    let binding = Binding::from_resource(&object(role_binding("view", None))).expect("binding");
    let role = role_of(&h.manager, &id("a"), &binding)
        .now_or_never()
        .expect("at once")
        .expect("read");
    assert_eq!(role.map(|r| r.id.name.to_string()), Some("view".to_owned()));
    let gone = Binding::from_resource(&object(role_binding("gone", None))).expect("binding");
    assert!(
        role_of(&h.manager, &id("a"), &gone)
            .now_or_never()
            .expect("at once")
            .expect("read")
            .is_none()
    );
}

/// A reader that counts the lists in flight: each list yields once before it answers, so lists
/// run one after another never overlap.
struct Overlap {
    inner: Arc<FakeResourcePort>,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
    full_lists: AtomicUsize,
}

impl Overlap {
    async fn enter(&self) {
        let now = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                std::task::Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                std::task::Poll::Pending
            }
        })
        .await;
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl ResourceReader for Overlap {
    async fn list(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        self.full_lists.fetch_add(1, Ordering::SeqCst);
        self.enter().await;
        self.inner.list(kind, namespace, options).await
    }

    async fn list_metadata(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        self.enter().await;
        self.inner.list_metadata(kind, namespace, options).await
    }

    async fn get(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Resource> {
        self.inner.get(kind, namespace, name).await
    }

    async fn get_opt(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> OxiResult<Option<Resource>> {
        self.inner.get_opt(kind, namespace, name).await
    }

    async fn watch(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &WatchOptions,
    ) -> OxiResult<WatchFeed<Resource>> {
        self.inner.watch(kind, namespace, options).await
    }

    async fn get_scale(&self, kind: &Gvk, namespace: Option<&str>, name: &str) -> OxiResult<Scale> {
        self.inner.get_scale(kind, namespace, name).await
    }

    async fn get_subresource(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        name: &str,
        subresource: &Subresource,
    ) -> OxiResult<serde_json::Value> {
        self.inner
            .get_subresource(kind, namespace, name, subresource)
            .await
    }
}

/// The dialog's five lists run at once, and only the bindings are read as whole objects.
#[test]
fn the_lists_run_concurrently_and_names_come_from_metadata() {
    let h = seeded();
    let overlap = Arc::new(Overlap {
        inner: h.resources("a"),
        in_flight: Default::default(),
        peak: Default::default(),
        full_lists: Default::default(),
    });
    let reader: Arc<dyn ResourceReader> = overlap.clone();
    let choices = futures::executor::block_on(BindingChoices::read_from(&reader));
    assert_eq!(choices.roles.len(), 2);
    assert_eq!(choices.cluster_roles.len(), 1);
    assert_eq!(choices.service_accounts.len(), 1);
    assert_eq!(
        overlap.peak.load(Ordering::SeqCst),
        5,
        "all five lists in flight together"
    );
    assert_eq!(
        overlap.full_lists.load(Ordering::SeqCst),
        2,
        "only the two binding kinds are read whole"
    );
}
