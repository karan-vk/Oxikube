//! The four kinds register through their own modules, in the Workloads section's order.

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::Gvk;

use crate::kinds::{DetailRenderer, KindSpecs};
use crate::table::tests::fixture::Fixture;

fn registered(f: &mut Fixture) {
    f.vcx.update(|_, cx| crate::kinds::init(cx));
}

#[gpui::test]
fn the_four_kinds_offer_scale_and_a_panel_and_keep_their_sidebar_places(cx: &mut TestAppContext) {
    let mut f = Fixture::new(cx);
    registered(&mut f);
    f.vcx.update(|_, cx| {
        let section = KindSpecs::registry(cx);
        let tree = section.sidebar_tree(None);
        let workloads = tree
            .sections
            .iter()
            .find(|s| s.section.id == "workloads")
            .expect("the Workloads section");
        let order: Vec<_> = workloads
            .entries
            .iter()
            .map(|e| &*e.meta.gvk.kind)
            .collect();
        assert_eq!(
            order,
            [
                "Pod",
                "Deployment",
                "DaemonSet",
                "StatefulSet",
                "ReplicaSet",
                "ReplicationController",
                "Job",
                "CronJob"
            ],
            "the order does not depend on who registers what"
        );
        for kind in [
            ("apps", "Deployment"),
            ("apps", "StatefulSet"),
            ("apps", "ReplicaSet"),
            ("", "ReplicationController"),
        ] {
            let gvk = Gvk::new(kind.0, "v1", kind.1);
            let meta = KindSpecs::kind_for(cx, &gvk);
            assert!(meta.offers(CommandId::WORKLOAD_SCALE), "{}", kind.1);
            assert!(
                matches!(KindSpecs::detail_for(cx, &gvk), DetailRenderer::Panel(_)),
                "{}",
                kind.1
            );
        }
        // The kinds that keep a revision history roll back (E12-S06); a ReplicationController has none.
        for (kind, rolls_back) in [
            (("apps", "Deployment"), true),
            (("apps", "StatefulSet"), true),
            (("apps", "ReplicaSet"), true),
            (("", "ReplicationController"), false),
        ] {
            let gvk = Gvk::new(kind.0, "v1", kind.1);
            assert_eq!(
                KindSpecs::kind_for(cx, &gvk).offers(CommandId::WORKLOAD_ROLLBACK),
                rolls_back,
                "{}",
                kind.1
            );
        }
        // A DaemonSet cannot be scaled (no action); its own panel is E12-S07's (`kinds::daemonset`).
        let daemonset = Gvk::new("apps", "v1", "DaemonSet");
        assert!(!KindSpecs::kind_for(cx, &daemonset).offers(CommandId::WORKLOAD_SCALE));
        assert!(matches!(
            KindSpecs::detail_for(cx, &daemonset),
            DetailRenderer::Panel(_)
        ));
        // Restart and Set image (E12-S05): Deployments, StatefulSets and DaemonSets, not the
        // controllers whose pods a template change never rolls.
        for (group, kind, offered) in [
            ("apps", "Deployment", true),
            ("apps", "StatefulSet", true),
            ("apps", "DaemonSet", true),
            ("apps", "ReplicaSet", false),
            ("", "ReplicationController", false),
        ] {
            let meta = KindSpecs::kind_for(cx, &Gvk::new(group, "v1", kind));
            for command in [CommandId::WORKLOAD_RESTART, CommandId::WORKLOAD_SET_IMAGE] {
                assert_eq!(meta.offers(command), offered, "{kind} {command}");
            }
        }
        // Pods keep theirs.
        let pod = Gvk::new("", "v1", "Pod");
        assert!(!KindSpecs::kind_for(cx, &pod).offers(CommandId::WORKLOAD_SCALE));
        assert!(KindSpecs::kind_for(cx, &pod).offers(CommandId::POD_SHELL));
    });
}
