//! The pod panel in a pod's detail: the action bar, the sections, the owner chain read once, the
//! environment read when it is opened with its references resolved and Secret values masked, and
//! the reveal toggle.

use gpui::{Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::view::{MASK, SecretValue};
use oxikube_testkit::ResourceCall;
use serde_json::json;

use super::{EnvCell, EnvLine, PodPanel, Row, SectionId};
use crate::detail::DetailView;
use crate::detail::tests::fixture::{Detail, edited, pod_ref, web_pod, web_replicaset};
use crate::table::tests::fixture::cluster;

/// `shop/web-0` with an environment: one `envFrom`, a Secret key, a missing ConfigMap key, a
/// field reference; a mount, a probe, a volume, a toleration and an affinity.
fn env_pod() -> Resource {
    edited(web_pod(), |json| {
        json["spec"] = json!({
            "nodeName": "worker-1",
            "nodeSelector": {"disktype": "ssd"},
            "tolerations": [{"key": "dedicated", "operator": "Exists", "effect": "NoSchedule"}],
            "volumes": [{"name": "cfg", "configMap": {"name": "web-config"}}],
            "containers": [{
                "name": "app", "image": "web:1",
                "envFrom": [{"configMapRef": {"name": "web-config"}}],
                "env": [
                    {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "web-secret", "key": "token"}}},
                    {"name": "GONE", "valueFrom": {"configMapKeyRef": {"name": "missing", "key": "k"}}},
                    {"name": "POD_IP", "valueFrom": {"fieldRef": {"fieldPath": "status.podIP"}}},
                ],
                "volumeMounts": [{"name": "cfg", "mountPath": "/etc/web", "readOnly": true}],
                "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080}},
            }],
        });
        json["status"]["podIP"] = json!("10.1.2.3");
    })
}

fn config_map() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "web-config", "namespace": "shop"},
        "data": {"mode": "prod"},
    }))
    .unwrap()
}

fn secret() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "web-secret", "namespace": "shop"},
        "data": {"token": "aHVudGVyMg=="},
    }))
    .unwrap()
}

fn deployment() -> Resource {
    Resource::from_json(json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop", "resourceVersion": "2"},
    }))
    .unwrap()
}

/// A replica set owned by the Deployment `web`.
fn owned_replicaset() -> Resource {
    edited(web_replicaset(), |json| {
        json["metadata"]["ownerReferences"] = json!([{
            "apiVersion": "apps/v1", "kind": "Deployment", "name": "web",
            "uid": "dep-uid", "controller": true
        }]);
    })
}

fn open(cx: &mut TestAppContext) -> (Detail, Entity<DetailView>) {
    open_with(cx, env_pod())
}

/// Like [`open`], with `pod` as `shop/web-0`.
fn open_with(cx: &mut TestAppContext, pod: Resource) -> (Detail, Entity<DetailView>) {
    let mut d = Detail::new(
        cx,
        [
            pod,
            config_map(),
            secret(),
            owned_replicaset(),
            deployment(),
        ],
    );
    d.f.vcx.update(|_, cx| crate::kinds::init(cx));
    let view = d.open(&pod_ref("web-0"));
    (d, view)
}

fn panel(d: &mut Detail, view: &Entity<DetailView>) -> Entity<PodPanel> {
    d.read(view, |v| v.kind_panel().cloned())
        .expect("the pod's panel")
        .downcast::<PodPanel>()
        .expect("the pod panel")
}

fn rows_of(d: &mut Detail, panel: &Entity<PodPanel>, id: SectionId) -> Vec<Row> {
    d.f.vcx.update(|_, cx| {
        panel
            .read(cx)
            .sections()
            .iter()
            .find(|s| s.id == id)
            .map(|s| s.rows.clone())
            .unwrap_or_default()
    })
}

fn gets_of_config_and_secrets(d: &mut Detail) -> Vec<String> {
    d.f.ports()
        .resources
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            ResourceCall::GetOpt { kind, name, .. }
                if matches!(&*kind.kind, "ConfigMap" | "Secret") =>
            {
                Some(format!("{}/{name}", kind.kind))
            }
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn a_pods_detail_has_the_actions_the_containers_and_the_short_sections(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    for selector in [
        "pod-panel",
        "pod-actions",
        "pod-action-logs",
        "pod-action-evict",
        "pod-action-delete",
        "pod-action-force-delete",
        "pod-action-jump-to-owner",
        "pod-action-show-node",
        "pod-containers",
        "pod-section-owners",
        "pod-section-node",
        "pod-section-environment",
        "pod-section-mounts",
        "pod-section-probes",
        "pod-section-volumes",
        "pod-section-tolerations",
        "pod-section-affinity",
        "pod-link-node-0",
    ] {
        assert!(d.shown(selector), "{selector} is on screen");
    }
    assert!(
        d.shown("detail-overview"),
        "the generic Overview stays below"
    );
    // The long sections start collapsed: their rows are not even built.
    for selector in [
        "pod-row-environment-0",
        "pod-row-mounts-0",
        "pod-row-volumes-0",
        "pod-row-tolerations-0",
    ] {
        assert!(!d.shown(selector), "{selector} starts collapsed");
    }
    let panel = panel(&mut d, &view);
    d.f.vcx.update(|_, cx| {
        let panel = panel.read(cx);
        assert!(panel.is_open(SectionId::Owners) && panel.is_open(SectionId::Node));
        assert!(!panel.is_open(SectionId::Environment));
        let counts: Vec<_> = panel
            .sections()
            .iter()
            .map(|s| (s.id.key(), s.count))
            .collect();
        assert_eq!(
            counts,
            [
                ("owners", 2),
                ("node", 1),
                ("environment", 4),
                ("mounts", 1),
                ("probes", 1),
                ("volumes", 1),
                ("tolerations", 1),
                ("affinity", 1),
            ]
        );
    });
}

#[gpui::test]
fn the_owner_chain_is_read_once_and_each_level_opens_its_detail(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    let panel = panel(&mut d, &view);
    let rows = rows_of(&mut d, &panel, SectionId::Owners);
    let links: Vec<(String, String)> = rows
        .iter()
        .filter_map(|row| match row {
            Row::Link { label, text, .. } => Some((label.clone(), text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        [
            ("Owned by".to_owned(), "ReplicaSet web-5d".to_owned()),
            ("…owned by".to_owned(), "Deployment web".to_owned()),
        ]
    );
    assert!(d.shown("pod-link-owners-1"));

    // A click on the Deployment opens its detail (`resource::Open`).
    d.f.dispatcher.clear();
    d.click("pod-link-owners-1");
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceOpen { target }]
            if &*target.gvk.kind == "Deployment" && &*target.name == "web"
                && target.namespace() == Some("shop")),
        "{sent:?}"
    );
    let shown = d.drawer_view().expect("the drawer follows the link");
    assert_eq!(&*d.read(&shown, |v| v.target().name.clone()), "web");
}

#[gpui::test]
fn the_node_row_links_to_the_node(cx: &mut TestAppContext) {
    let (mut d, _view) = open(cx);
    d.f.dispatcher.clear();
    d.click("pod-link-node-0");
    let sent = d.f.dispatcher.sent();
    assert!(
        matches!(sent.as_slice(), [Command::ResourceOpen { target }]
            if &*target.gvk.kind == "Node" && &*target.name == "worker-1" && target.namespace().is_none()),
        "{sent:?}"
    );
}

#[gpui::test]
fn nothing_is_read_for_the_environment_until_it_is_opened_and_then_once_per_object(
    cx: &mut TestAppContext,
) {
    let (mut d, view) = open(cx);
    assert!(
        gets_of_config_and_secrets(&mut d).is_empty(),
        "a Secret is not read just because a pod is open"
    );
    d.click("pod-section-environment");
    let mut gets = gets_of_config_and_secrets(&mut d);
    gets.sort();
    assert_eq!(
        gets,
        [
            "ConfigMap/missing",
            "ConfigMap/web-config",
            "Secret/web-secret"
        ],
        "one get per object"
    );
    // Closing and opening it again reads nothing more.
    d.click("pod-section-environment");
    d.click("pod-section-environment");
    assert_eq!(gets_of_config_and_secrets(&mut d).len(), 3);
    let _ = view;
}

#[gpui::test]
fn the_environment_resolves_references_and_masks_secret_values(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    d.click("pod-section-environment");
    let panel = panel(&mut d, &view);
    let rows = rows_of(&mut d, &panel, SectionId::Environment);
    let env = |name: &str| {
        rows.iter()
            .find_map(|row| match row {
                Row::Env(EnvLine {
                    name: n,
                    cell,
                    origin,
                    ..
                }) if n == name => Some((cell.clone(), origin.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no variable {name}"))
    };
    // envFrom: a ConfigMap key shows its value.
    assert_eq!(env("mode").0, EnvCell::Plain("prod".into()));
    // A Secret key is a Secret value (masked on screen until revealed).
    let (token, origin) = env("TOKEN");
    assert_eq!(token, EnvCell::Secret(SecretValue::new("hunter2")));
    assert_eq!(origin, "Secret web-secret · token");
    // A missing ConfigMap is a chip, not an error.
    assert!(matches!(
        env("GONE").0,
        EnvCell::Chip { ref text, .. } if text == "ConfigMap missing not found"
    ));
    // A field reference resolves from the pod.
    assert_eq!(env("POD_IP").0, EnvCell::Plain("10.1.2.3".into()));

    assert!(d.shown("pod-env-2-chip"));
    assert!(d.shown("pod-env-1-value"));
    assert!(d.shown("pod-env-1-reveal"));
    assert!(
        !d.shown("pod-env-0-reveal"),
        "only Secret values have the toggle"
    );
}

#[gpui::test]
fn a_pod_with_only_literal_and_field_variables_shows_them_without_reading_anything(
    cx: &mut TestAppContext,
) {
    let pod = edited(web_pod(), |json| {
        json["spec"] = json!({
            "nodeName": "worker-1",
            "containers": [{
                "name": "app", "image": "web:1",
                "env": [
                    {"name": "MODE", "value": "prod"},
                    {"name": "POD_IP", "valueFrom": {"fieldRef": {"fieldPath": "status.podIP"}}},
                ],
            }],
        });
        json["status"]["podIP"] = json!("10.1.2.3");
    });
    let (mut d, view) = open_with(cx, pod);
    d.click("pod-section-environment");
    let panel = panel(&mut d, &view);
    let rows = rows_of(&mut d, &panel, SectionId::Environment);
    let cell = |name: &str| {
        rows.iter().find_map(|row| match row {
            Row::Env(EnvLine { name: n, cell, .. }) if n == name => Some(cell.clone()),
            _ => None,
        })
    };
    assert_eq!(cell("MODE"), Some(EnvCell::Plain("prod".into())));
    assert_eq!(cell("POD_IP"), Some(EnvCell::Plain("10.1.2.3".into())));
    assert!(
        gets_of_config_and_secrets(&mut d).is_empty(),
        "nothing to read: no ConfigMap or Secret is named"
    );
}

#[gpui::test]
fn the_reveal_button_toggles_one_secret_and_nothing_else(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    d.click("pod-section-environment");
    let panel = panel(&mut d, &view);
    let key = (0, "TOKEN".to_owned());
    let rows = rows_of(&mut d, &panel, SectionId::Environment);
    let secret = rows
        .iter()
        .find_map(|row| match row {
            Row::Env(EnvLine {
                cell: EnvCell::Secret(value),
                ..
            }) => Some(value.clone()),
            _ => None,
        })
        .expect("a Secret variable");
    // What the line draws for the value: the mask until revealed, never the decoded text.
    let drawn = |d: &mut Detail| {
        d.f.vcx
            .update(|_, cx| panel.read(cx).secret_text(&key, &secret).to_owned())
    };
    assert!(!d.f.vcx.update(|_, cx| panel.read(cx).is_revealed(&key)));
    assert_eq!(drawn(&mut d), MASK);
    d.click("pod-env-1-reveal");
    assert!(d.f.vcx.update(|_, cx| panel.read(cx).is_revealed(&key)));
    assert_eq!(drawn(&mut d), "hunter2");
    d.click("pod-env-1-reveal");
    assert!(!d.f.vcx.update(|_, cx| panel.read(cx).is_revealed(&key)));
    assert_eq!(drawn(&mut d), MASK);
    // Revealing reads nothing more, writes nothing, and sends no command.
    assert_eq!(gets_of_config_and_secrets(&mut d).len(), 3);
    assert!(d.f.ports().resources.mutating_calls().is_empty());
}

#[gpui::test]
fn the_other_sections_list_mounts_probes_volumes_tolerations_and_affinity(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    let panel = panel(&mut d, &view);
    // (The panel scrolls, so the lower headers are opened through the entity.)
    for id in [
        SectionId::Mounts,
        SectionId::Probes,
        SectionId::Volumes,
        SectionId::Tolerations,
        SectionId::Affinity,
    ] {
        d.f.vcx
            .update(|_, cx| panel.update(cx, |panel, cx| panel.toggle(id, cx)));
    }
    d.settle();
    let cells = |rows: Vec<Row>| -> Vec<Vec<String>> {
        rows.into_iter()
            .filter_map(|row| match row {
                Row::Cells(cells) => Some(cells),
                _ => None,
            })
            .collect()
    };
    let mounts = cells(rows_of(&mut d, &panel, SectionId::Mounts));
    assert_eq!(mounts, [["/etc/web", "cfg", "read-only"]]);
    let probes = cells(rows_of(&mut d, &panel, SectionId::Probes));
    assert_eq!(probes[0][0], "liveness");
    assert_eq!(probes[0][1], "http-get http://:8080/healthz");
    let volumes = cells(rows_of(&mut d, &panel, SectionId::Volumes));
    assert_eq!(volumes, [["cfg", "ConfigMap", "web-config"]]);
    let pairs: Vec<(String, String)> = [SectionId::Tolerations, SectionId::Affinity]
        .into_iter()
        .flat_map(|id| rows_of(&mut d, &panel, id))
        .filter_map(|row| match row {
            Row::Pair { label, value } => Some((label, value)),
            _ => None,
        })
        .collect();
    assert_eq!(
        pairs,
        [
            (
                "Tolerates".to_owned(),
                "dedicated:NoSchedule op=Exists".to_owned()
            ),
            ("Node selector".to_owned(), "disktype=ssd".to_owned()),
        ]
    );
}

#[gpui::test]
fn the_model_is_built_once_per_pod_version_not_per_frame(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    let panel = panel(&mut d, &view);
    let builds = d.f.vcx.update(|_, cx| panel.read(cx).builds());
    assert_eq!(builds, 1);
    for _ in 0..3 {
        d.update(&view, |_, cx| cx.notify());
        d.draw();
    }
    assert_eq!(d.f.vcx.update(|_, cx| panel.read(cx).builds()), builds);
}

fn bar_commands(d: &mut Detail, clicks: &[&'static str]) -> Vec<Command> {
    d.f.dispatcher.clear();
    for selector in clicks {
        d.click(selector);
    }
    d.f.dispatcher.sent()
}

#[gpui::test]
fn the_buttons_send_the_pod_commands_the_bus_guards(cx: &mut TestAppContext) {
    let (mut d, _view) = open(cx);
    let pod = pod_ref("web-0");
    let sent = bar_commands(
        &mut d,
        &[
            "pod-action-evict",
            "pod-action-delete",
            "pod-action-force-delete",
            "pod-action-logs",
        ],
    );
    assert_eq!(sent.len(), 4, "{sent:?}");
    assert_eq!(
        sent[0],
        Command::PodEvict {
            target: pod.clone()
        }
    );
    assert_eq!(
        sent[1],
        Command::PodDelete {
            target: pod.clone(),
            grace_period_seconds: None
        }
    );
    assert_eq!(sent[2], Command::PodForceDelete { target: pod });
    assert_eq!(sent[3].id(), CommandId::POD_VIEW_LOGS);
}

#[gpui::test]
fn jump_to_owner_and_show_node_open_the_object_they_read(cx: &mut TestAppContext) {
    let (mut d, _view) = open(cx);
    d.click("pod-action-jump-to-owner");
    d.settle();
    let shown = d.drawer_view().expect("the drawer opened");
    let target = d.read(&shown, |v| v.target().clone());
    assert_eq!(
        (&*target.gvk.kind, &*target.name),
        ("ReplicaSet", "web-5d"),
        "the pod's controller"
    );

    // Back on the pod: its node.
    d.open(&pod_ref("web-0"));
    d.click("pod-action-show-node");
    d.settle();
    let shown = d.drawer_view().expect("the drawer opened");
    let target = d.read(&shown, |v| v.target().clone());
    assert_eq!(
        (&*target.gvk.kind, &*target.name, target.namespace()),
        ("Node", "worker-1", None)
    );
}

#[gpui::test]
fn a_read_only_cluster_greys_out_the_mutating_buttons_and_keeps_the_reads(cx: &mut TestAppContext) {
    let (mut d, view) = open(cx);
    d.f.sessions.set_read_only(&cluster(), true).unwrap();
    d.settle();
    d.update(&view, |_, cx| cx.notify());
    d.settle();
    let sent = bar_commands(
        &mut d,
        &[
            "pod-action-evict",
            "pod-action-delete",
            "pod-action-force-delete",
            "pod-action-logs",
            "pod-action-show-node",
        ],
    );
    let ids: Vec<_> = sent.iter().map(Command::id).collect();
    assert_eq!(
        ids,
        [CommandId::POD_VIEW_LOGS, CommandId::POD_SHOW_NODE],
        "a blocked button sends nothing"
    );
    let reason = d.read(&view, |v| v.command_block(CommandId::POD_EVICT));
    assert_eq!(reason, Some(Some("This cluster is read-only".to_owned())));
}
