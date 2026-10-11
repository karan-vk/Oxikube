//! Tests of the Config kinds' panels: the builders as plain tests over the testkit fixtures
//! (`panels`), and the registered kinds in the window (`views`).

mod panels;
mod views;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_domain::Resource;
use oxikube_domain::access::AccessRules;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::kinds::ResourceKind;
use oxikube_testkit::discovery_sets;

use crate::detail::tests::fixture::Detail;
use crate::table::tests::fixture::{Fixture, cluster};

/// A window with the built-in kinds registered, connected to a cluster serving the stock kinds
/// and the VPA CRD, which holds `objects`.
pub(super) fn window(
    cx: &mut TestAppContext,
    objects: impl IntoIterator<Item = Resource>,
) -> Detail {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.access.set_rules(AccessRules::all_access());
    ports.discovery.set_kinds(discovery_sets::core_with_vpa());
    for object in objects {
        ports.resources.insert(object);
    }
    block_on(f.sessions.connect(&cluster())).expect("connect");
    f.settle();
    Detail { f }
}

/// The discovery entry of `kind`.
pub(super) fn served(kind: &str) -> ResourceKind {
    discovery_sets::core_with_vpa()
        .into_iter()
        .find(|k| &*k.gvk.kind == kind)
        .unwrap_or_else(|| panic!("{kind} is served"))
}

/// The reference of `res` in the test cluster.
pub(super) fn reference(res: &Resource) -> ResourceRef {
    ResourceRef::new(
        cluster(),
        res.kind.clone(),
        res.meta.namespace.clone(),
        res.meta.name.clone(),
    )
}
