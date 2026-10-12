//! Node tests (E12-S09): the table's columns and its live Pods column, the cordon badge after a
//! patch (`table`), the detail panel (`panel`), the actions and keys (`actions`) and the drain
//! dialog (`drain`, E12-S10). All
//! `#[gpui::test]`s over testkit fakes, through the real bus where a command is sent.

mod actions;
mod allocation;
mod drain;
mod panel;
mod table;

use std::time::Duration;

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_testkit::{node, pod};

use super::panel::NodePanel;
use crate::detail::DetailView;
use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::cluster;

pub(super) fn node_ref(name: &str) -> ResourceRef {
    ResourceRef::cluster_scoped(cluster(), Gvk::new("", "v1", "Node"), name)
}

/// A running pod `x/name` on `node` requesting `(cpu, memory)` and limited to the second pair.
pub(super) fn pod_on(
    name: &str,
    node: &str,
    requests: (&str, &str),
    limits: (&str, &str),
) -> Resource {
    pod()
        .namespace("x")
        .name(name)
        .node(node)
        .running()
        .requests(requests.0, requests.1)
        .limits(limits.0, limits.1)
        .build()
}

/// A detail window over `objects` with the built-in kinds registered (the node spec among them).
pub(super) fn node_detail(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = Resource>,
) -> Detail {
    let d = Detail::new(cx, objects);
    let mut d = d;
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    d
}

/// The panel of the detail `view` shows (drawn once so it exists).
pub(super) fn panel_of(d: &mut Detail, view: &Entity<DetailView>) -> Entity<NodePanel> {
    d.draw();
    let any = d.read(view, |v| v.kind_panel().cloned().expect("the node panel"));
    super::panel::of(&any)
}

/// Lets the panel's timer fire and what it started land.
pub(super) fn tick(d: &mut Detail) {
    d.f.vcx.executor().advance_clock(Duration::from_millis(300));
    d.settle();
}

/// A node worth looking at: 4 CPU / 16 GiB / 110 pods allocatable.
pub(super) fn worker() -> Resource {
    node().name("worker-1").build()
}

/// The text of the CPU requests line of the panel's allocation section.
pub(super) fn cpu_requests(d: &mut Detail, panel: &Entity<NodePanel>) -> String {
    use oxikube_app::allocation::AllocationKind;
    d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .allocation_view()
            .and_then(|view| view.row(&AllocationKind::Cpu))
            .map(|row| row.requests_text())
            .expect("the allocation is built")
    })
}
