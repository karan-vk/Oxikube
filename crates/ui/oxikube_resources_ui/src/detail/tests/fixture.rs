//! The window every detail test starts from: [`Fixture`] of the table tests (cluster tabs with
//! the sidebar, the `ResourceViews` controller), plus the kinds and objects of the test.

use futures::executor::block_on;
use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_workspace::Workspace;
use serde_json::{Value, json};

use crate::detail::{DetailDrawer, DetailView};
use crate::table::tests::fixture::{Fixture, cluster};

/// A kind as discovery serves it.
pub(crate) fn kind(
    group: &str,
    version: &str,
    name: &str,
    plural: &str,
    namespaced: bool,
) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, version, name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: Vec::new(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced,
    }
}

/// The kinds the tests serve: pods, replica sets, deployments, secrets, nodes, the Widget custom
/// kind and the CRD kind.
pub(crate) fn kinds() -> Vec<ResourceKind> {
    vec![
        kind("", "v1", "Pod", "pods", true),
        kind("apps", "v1", "ReplicaSet", "replicasets", true),
        kind("apps", "v1", "Deployment", "deployments", true),
        kind("", "v1", "Secret", "secrets", true),
        kind("", "v1", "Node", "nodes", false),
        kind("example.com", "v1", "Widget", "widgets", true),
        kind(
            "apiextensions.k8s.io",
            "v1",
            "CustomResourceDefinition",
            "customresourcedefinitions",
            false,
        ),
    ]
}

/// `base` with `edit` applied to its JSON.
pub(crate) fn edited(base: Resource, edit: impl FnOnce(&mut Value)) -> Resource {
    let mut json = base.json;
    edit(&mut json);
    Resource::from_json(json).expect("still a resource")
}

/// A pod `shop/web-0` owned by the replica set `web-5d`, with labels, an annotation and
/// conditions.
pub(crate) fn web_pod() -> Resource {
    edited(
        oxikube_testkit::pod()
            .namespace("shop")
            .name("web-0")
            .label("app", "web")
            .label("tier", "frontend")
            .annotation("note", "hello")
            .created("2026-01-01T00:00:00Z")
            .build(),
        |json| {
            json["metadata"]["resourceVersion"] = json!("7");
            json["metadata"]["ownerReferences"] = json!([{
                "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d",
                "uid": "rs-uid", "controller": true, "blockOwnerDeletion": true
            }]);
            json["metadata"]["finalizers"] = json!(["example.com/cleanup"]);
            json["status"]["conditions"] = json!([
                {"type": "Ready", "status": "True", "lastTransitionTime": "2026-01-01T00:00:10Z"},
                {"type": "PodScheduled", "status": "True", "reason": "Scheduled",
                 "message": "assigned to node", "lastTransitionTime": "2026-01-01T00:00:05Z"}
            ]);
        },
    )
}

/// The replica set that owns [`web_pod`].
pub(crate) fn web_replicaset() -> Resource {
    let mut rs = oxikube_testkit::replicaset()
        .namespace("shop")
        .name("web-5d")
        .build();
    rs.meta.resource_version = Some("3".into());
    rs
}

pub(crate) fn pod_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", name)
}

/// One window with a connected cluster `kind`.
pub(crate) struct Detail {
    pub(crate) f: Fixture,
}

impl Detail {
    /// A window over a cluster that serves [`kinds`] and holds `objects`, connected.
    pub(crate) fn new(
        cx: &mut TestAppContext,
        objects: impl IntoIterator<Item = Resource>,
    ) -> Self {
        Self::over(Fixture::new(cx), objects)
    }

    /// [`Self::new`] over the fixture with the exec row actions (E09-S08).
    pub(crate) fn with_exec(
        cx: &mut TestAppContext,
        objects: impl IntoIterator<Item = Resource>,
    ) -> Self {
        Self::over(Fixture::with_exec(cx), objects)
    }

    fn over(f: Fixture, objects: impl IntoIterator<Item = Resource>) -> Self {
        let ports = f.ports();
        ports.discovery.set_kinds(kinds());
        for object in objects {
            ports.resources.insert(object);
        }
        block_on(f.sessions.connect(&cluster())).expect("connect");
        f.vcx.run_until_parked();
        Self { f }
    }

    /// The cluster tab's workspace.
    pub(crate) fn workspace(&mut self) -> Entity<Workspace> {
        let tabs = self.f.tabs.clone();
        self.f.vcx.update(|_, cx| {
            let tab = tabs.read(cx).tab(&cluster()).cloned().expect("a tab");
            tab.read(cx).workspace().clone()
        })
    }

    /// Opens the detail of `target` as `resource::Open` does.
    pub(crate) fn open(&mut self, target: &ResourceRef) -> Entity<DetailView> {
        let views = self.f.views.clone();
        let target = target.clone();
        let view = self
            .f
            .vcx
            .update(|window, cx| {
                views.update(cx, |views, cx| views.open_detail(&target, window, cx))
            })
            .expect("the cluster has a tab");
        self.settle();
        view
    }

    /// The drawer of the cluster tab.
    pub(crate) fn drawer(&mut self) -> Option<Entity<DetailDrawer>> {
        let workspace = self.workspace();
        self.f
            .vcx
            .update(|_, cx| workspace.read(cx).panel::<DetailDrawer>())
    }

    /// The detail the drawer shows.
    pub(crate) fn drawer_view(&mut self) -> Option<Entity<DetailView>> {
        let drawer = self.drawer()?;
        self.f.vcx.update(|_, cx| drawer.read(cx).view().cloned())
    }

    /// Lets feeds, reads and a coalesced redraw land.
    pub(crate) fn settle(&mut self) {
        self.f.settle();
        self.f.settle();
    }

    /// Draws a frame.
    pub(crate) fn draw(&mut self) {
        self.f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    }

    /// Whether the element tagged `selector` is on screen.
    pub(crate) fn shown(&mut self, selector: &'static str) -> bool {
        self.draw();
        self.f.vcx.debug_bounds(selector).is_some()
    }

    /// Clicks the element tagged `selector` and lets the effects land.
    pub(crate) fn click(&mut self, selector: &'static str) {
        self.draw();
        let bounds = self
            .f
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is on screen"));
        self.f
            .vcx
            .simulate_click(bounds.center(), Default::default());
        self.settle();
    }

    /// Runs `f` on `view`.
    pub(crate) fn update<R>(
        &mut self,
        view: &Entity<DetailView>,
        f: impl FnOnce(&mut DetailView, &mut gpui::Context<DetailView>) -> R,
    ) -> R {
        let result = self.f.vcx.update(|_, cx| view.update(cx, f));
        self.settle();
        result
    }

    /// Reads `view`.
    pub(crate) fn read<R>(
        &mut self,
        view: &Entity<DetailView>,
        f: impl FnOnce(&DetailView) -> R,
    ) -> R {
        self.f.vcx.update(|_, cx| f(view.read(cx)))
    }
}
