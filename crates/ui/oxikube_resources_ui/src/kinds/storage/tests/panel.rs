//! The detail panels.

use gpui::TestAppContext;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_testkit::storage::{pod_mounting, pvc, storage_class};
use serde_json::json;

use super::{kind, open_detail, storage_window, wait};
use crate::kinds::storage::panel::StoragePanel;
use crate::table::tests::fixture::cluster;

fn claim_ref(name: &str) -> ResourceRef {
    ResourceRef::namespaced(cluster(), kind("persistentvolumeclaims").gvk, "shop", name)
}

/// The panel entity of the detail `view` shows.
fn panel_of(
    f: &mut crate::table::tests::fixture::Fixture,
    view: &gpui::Entity<crate::detail::DetailView>,
) -> gpui::Entity<StoragePanel> {
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    f.vcx.update(|_, cx| {
        view.read(cx)
            .kind_panel()
            .expect("the kind has a panel")
            .clone()
            .downcast::<StoragePanel>()
            .expect("a storage panel")
    })
}

fn texts(panel: &StoragePanel) -> Vec<(String, String)> {
    panel
        .content()
        .fields
        .iter()
        .map(|f| (f.label.to_owned(), f.value.clone()))
        .collect()
}

#[gpui::test]
fn the_claim_detail_shows_its_bound_volume_and_the_pods_that_mount_it(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data")
                .namespace("shop")
                .class("fast")
                .requests("10Gi")
                .build(),
            storage_class("fast").expandable(true).build(),
            pod_mounting("web-1", "shop", &["data"]),
            pod_mounting("web-0", "shop", &["data"]),
            pod_mounting("other", "shop", &["elsewhere"]),
        ],
    );
    let view = open_detail(&mut f, &claim_ref("data"));
    let panel = panel_of(&mut f, &view);
    wait(&mut f, 1);
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let (fields, usage) = f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (texts(panel), panel.usage_block().cloned())
    });
    assert!(
        fields.contains(&("Status".into(), "Bound".into())),
        "{fields:?}"
    );
    assert!(fields.contains(&("Storage class".into(), "fast".into())));
    assert!(fields.contains(&("Requested".into(), "10Gi".into())));
    assert!(fields.contains(&("Capacity".into(), "10Gi".into())));
    assert!(fields.contains(&("Bound volume".into(), "pvc-0f3b6e0e".into())));
    let usage = usage.expect("a claim lists the pods that mount it");
    let pods: Vec<&str> = usage.lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(
        pods,
        ["web-0", "web-1"],
        "sorted; not the pod of another claim"
    );
    assert!(f.vcx.debug_bounds("storage-block-Mounted by").is_some());
    assert!(f.vcx.debug_bounds("storage-line-Mounted by-0").is_some());
}

#[gpui::test]
fn a_claim_nothing_mounts_says_so(cx: &mut TestAppContext) {
    let mut f = storage_window(cx, vec![pvc("data").namespace("shop").build()]);
    let view = open_detail(&mut f, &claim_ref("data"));
    let panel = panel_of(&mut f, &view);
    wait(&mut f, 1);
    let usage = f
        .vcx
        .update(|_, cx| panel.read(cx).usage_block().cloned())
        .unwrap();
    assert_eq!(usage.lines[0].text, "No pod mounts this claim.");
    assert!(usage.note);
}

#[gpui::test]
fn clicking_a_mounting_pod_or_the_bound_volume_opens_its_detail(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data").namespace("shop").build(),
            pod_mounting("web-0", "shop", &["data"]),
        ],
    );
    let view = open_detail(&mut f, &claim_ref("data"));
    let _panel = panel_of(&mut f, &view);
    wait(&mut f, 1);
    f.dispatcher.clear();
    f.vcx.update(|window, cx| window.draw(cx).clear(cx));
    let at = f
        .vcx
        .debug_bounds("storage-line-Mounted by-0")
        .expect("the pod line is on screen")
        .center();
    f.vcx.simulate_click(at, Default::default());
    f.settle();
    let opened: Vec<ResourceRef> = f
        .dispatcher
        .sent()
        .into_iter()
        .filter_map(|c| match c {
            Command::ResourceOpen { target } => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(opened.len(), 1, "{opened:?}");
    assert_eq!(&*opened[0].gvk.kind, "Pod");
    assert_eq!(&*opened[0].name, "web-0");
    assert_eq!(opened[0].namespace(), Some("shop"));
}

#[gpui::test]
fn a_pending_resize_is_on_the_claim_panel(cx: &mut TestAppContext) {
    let mut f = storage_window(
        cx,
        vec![
            pvc("data")
                .namespace("shop")
                .requests("20Gi")
                .capacity("10Gi")
                .condition("FileSystemResizePending", "True", "Waiting for the kubelet")
                .build(),
        ],
    );
    let view = open_detail(&mut f, &claim_ref("data"));
    let panel = panel_of(&mut f, &view);
    let fields = f.vcx.update(|_, cx| texts(panel.read(cx)));
    assert!(
        fields
            .iter()
            .any(|(l, v)| l == "Resize" && v.starts_with("10Gi → 20Gi")),
        "{fields:?}"
    );
}

#[gpui::test]
fn the_volume_detail_shows_its_claim_reclaim_policy_source_and_node_affinity(
    cx: &mut TestAppContext,
) {
    let volume = oxikube_testkit::resource("v1", "PersistentVolume")
        .cluster_scoped()
        .name("pv-1")
        .field(
            "spec",
            json!({
                "capacity": {"storage": "100Gi"}, "accessModes": ["ReadWriteOnce"],
                "persistentVolumeReclaimPolicy": "Delete",
                "claimRef": {"namespace": "shop", "name": "data"},
                "storageClassName": "fast", "csi": {"driver": "ebs.csi.aws.com", "volumeHandle": "vol-0"},
                "nodeAffinity": {"required": {"nodeSelectorTerms": [{"matchExpressions": [
                    {"key": "topology.kubernetes.io/zone", "operator": "In", "values": ["eu-1a"]}]}]}}
            }),
        )
        .field("status", json!({"phase": "Bound"}))
        .build();
    let mut f = storage_window(cx, vec![volume]);
    let target = ResourceRef::cluster_scoped(cluster(), kind("persistentvolumes").gvk, "pv-1");
    let view = open_detail(&mut f, &target);
    let panel = panel_of(&mut f, &view);
    let (fields, blocks) = f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (texts(panel), panel.content().blocks.clone())
    });
    assert!(
        fields.contains(&("Claim".into(), "shop/data".into())),
        "{fields:?}"
    );
    assert!(fields.contains(&("Reclaim policy".into(), "Delete".into())));
    assert!(fields.contains(&("Source".into(), "csi: ebs.csi.aws.com".into())));
    assert_eq!(blocks[0].title, "Node affinity");
    assert_eq!(
        blocks[0].lines[0].text,
        "topology.kubernetes.io/zone In eu-1a"
    );
}

#[gpui::test]
fn the_storage_class_detail_shows_parameters_and_the_default_flag(cx: &mut TestAppContext) {
    let class = {
        let mut json = storage_class("fast")
            .expandable(true)
            .default_class()
            .build()
            .to_value();
        json["parameters"] = json!({"type": "gp3", "encrypted": "true"});
        json["mountOptions"] = json!(["debug"]);
        oxikube_domain::Resource::from_json(json).unwrap()
    };
    let mut f = storage_window(cx, vec![class]);
    let target = ResourceRef::cluster_scoped(cluster(), kind("storageclasses").gvk, "fast");
    let view = open_detail(&mut f, &target);
    let panel = panel_of(&mut f, &view);
    let (fields, blocks) = f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        (texts(panel), panel.content().blocks.clone())
    });
    assert!(
        fields.contains(&("Provisioner".into(), "ebs.csi.aws.com".into())),
        "{fields:?}"
    );
    assert!(fields.contains(&("Default".into(), "Yes".into())));
    assert!(fields.contains(&("Allow expansion".into(), "Yes".into())));
    let parameters: Vec<&str> = blocks[0].lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(parameters, ["encrypted = true", "type = gp3"]);
    assert_eq!(blocks[1].title, "Mount options");
}
