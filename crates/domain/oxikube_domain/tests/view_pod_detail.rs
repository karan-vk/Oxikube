//! The pod detail view-model (E12-S03): environment resolution against ConfigMaps and Secrets,
//! mounts, probes, volumes, tolerations, affinity, node and owners.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_domain::view::{
    EnvSources, EnvValue, Missing, Origin, PodDetail, ProbeKind, SourceData, SourceKind,
    SourceState,
};
use serde_json::{Value, json};

fn pod(spec: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Pod",
        "metadata": {
            "name": "web-0", "namespace": "shop", "uid": "u-1",
            "labels": {"app": "web"}, "annotations": {"team": "core"},
            "ownerReferences": [{
                "apiVersion": "apps/v1", "kind": "ReplicaSet", "name": "web-5d",
                "uid": "rs", "controller": true
            }],
        },
        "spec": spec,
        "status": {"podIP": "10.1.2.3", "hostIP": "192.168.0.7",
                   "podIPs": [{"ip": "10.1.2.3"}, {"ip": "fd00::3"}]},
    }))
    .unwrap()
}

fn config_map(data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "web-config", "namespace": "shop"},
        "data": data, "binaryData": {"blob": "AAEC"},
    }))
    .unwrap()
}

fn secret(data: Value) -> Resource {
    Resource::from_json(json!({
        "apiVersion": "v1", "kind": "Secret",
        "metadata": {"name": "web-secret", "namespace": "shop"},
        "data": data,
    }))
    .unwrap()
}

fn sources(config: Option<&Resource>, secret_state: Option<SourceState>) -> EnvSources {
    let mut out = EnvSources::default();
    if let Some(config) = config {
        out.insert(
            SourceKind::ConfigMap,
            Arc::from("web-config"),
            SourceState::Found(SourceData::from_config_map(config)),
        );
    }
    if let Some(state) = secret_state {
        out.insert(SourceKind::Secret, Arc::from("web-secret"), state);
    }
    out
}

fn env_pod() -> Resource {
    pod(json!({
        "nodeName": "worker-1",
        "containers": [{
            "name": "app", "image": "web:1",
            "envFrom": [
                {"configMapRef": {"name": "web-config"}, "prefix": "CFG_"},
                {"secretRef": {"name": "web-secret", "optional": true}},
            ],
            "env": [
                {"name": "LITERAL", "value": "plain"},
                {"name": "URL", "valueFrom": {"configMapKeyRef": {"name": "web-config", "key": "url"}}},
                {"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "web-secret", "key": "token"}}},
                {"name": "GONE", "valueFrom": {"configMapKeyRef": {"name": "web-config", "key": "absent"}}},
                {"name": "OPT", "valueFrom": {"secretKeyRef": {"name": "nobody", "key": "k", "optional": true}}},
                {"name": "POD_IP", "valueFrom": {"fieldRef": {"fieldPath": "status.podIP"}}},
                {"name": "TEAM", "valueFrom": {"fieldRef": {"fieldPath": "metadata.annotations['team']"}}},
                {"name": "APP", "valueFrom": {"fieldRef": {"fieldPath": "metadata.labels['app']"}}},
                {"name": "IPS", "valueFrom": {"fieldRef": {"fieldPath": "status.podIPs"}}},
                {"name": "NOPE", "valueFrom": {"fieldRef": {"fieldPath": "status.nominatedNodeName"}}},
                {"name": "CPU_LIMIT", "valueFrom": {"resourceFieldRef": {"resource": "limits.cpu"}}},
            ],
            "resources": {"limits": {"cpu": "500m"}},
        }],
    }))
}

fn rows_of(pod: &Resource, sources: &EnvSources) -> Vec<oxikube_domain::view::EnvRow> {
    PodDetail::from_resource(pod).unwrap().containers[0].env_rows(pod, sources)
}

fn row<'a>(
    rows: &'a [oxikube_domain::view::EnvRow],
    name: &str,
) -> &'a oxikube_domain::view::EnvRow {
    rows.iter()
        .find(|r| r.name == name)
        .unwrap_or_else(|| panic!("no row {name}"))
}

#[test]
fn a_configmap_key_shows_its_value_and_a_secret_key_is_masked() {
    let config = config_map(json!({"url": "https://shop.example", "mode": "prod"}));
    // `dG9r` is base64 for `tok`.
    let secret = secret(json!({"token": "dG9r"}));
    let sources = sources(
        Some(&config),
        Some(SourceState::Found(SourceData::from_secret(&secret))),
    );
    let rows = rows_of(&env_pod(), &sources);

    assert_eq!(row(&rows, "LITERAL").value, EnvValue::Plain("plain".into()));
    assert_eq!(
        row(&rows, "URL").value,
        EnvValue::Plain("https://shop.example".into())
    );
    let token = row(&rows, "TOKEN");
    assert!(token.is_secret());
    let EnvValue::Secret(value) = &token.value else {
        panic!("a secret key is a Secret value");
    };
    assert_eq!(value.reveal(), "tok");
    // A secret value never prints, even through the whole row.
    assert!(!format!("{rows:?}").contains("tok\""));
    assert!(!format!("{token:?}").contains("\"tok\""));
}

#[test]
fn env_from_puts_the_prefix_before_every_key_and_binary_data_is_a_placeholder() {
    let config = config_map(json!({"url": "u", "mode": "prod"}));
    let rows = rows_of(&env_pod(), &sources(Some(&config), None));
    assert_eq!(row(&rows, "CFG_mode").value, EnvValue::Plain("prod".into()));
    assert_eq!(
        row(&rows, "CFG_blob").value,
        EnvValue::Plain("<binary>".into())
    );
    assert_eq!(
        row(&rows, "CFG_url").origin,
        Origin::Source {
            kind: SourceKind::ConfigMap,
            name: Arc::from("web-config"),
            key: Arc::from("url"),
        }
    );
    // The envFrom keys are sorted and come before the env entries.
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(&names[..3], ["CFG_blob", "CFG_mode", "CFG_url"]);
}

#[test]
fn an_env_entry_replaces_an_env_from_key_of_the_same_name() {
    let config = config_map(json!({"LITERAL": "from the map"}));
    let pod = pod(json!({"containers": [{
        "name": "app",
        "envFrom": [{"configMapRef": {"name": "web-config"}}],
        "env": [{"name": "LITERAL", "value": "wins"}],
    }]}));
    let rows = rows_of(&pod, &sources(Some(&config), None));
    let same: Vec<_> = rows.iter().filter(|r| r.name == "LITERAL").collect();
    assert_eq!(same.len(), 1);
    assert_eq!(same[0].value, EnvValue::Plain("wins".into()));
    assert_eq!(same[0].origin, Origin::Literal);
}

#[test]
fn missing_references_say_what_is_missing() {
    let config = config_map(json!({"url": "u"}));
    let sources = sources(Some(&config), Some(SourceState::NotFound));
    let rows = rows_of(&env_pod(), &sources);

    // The key is not in the ConfigMap.
    assert_eq!(
        row(&rows, "GONE").value,
        EnvValue::Missing(Missing::Key {
            kind: SourceKind::ConfigMap,
            name: Arc::from("web-config"),
            key: Arc::from("absent"),
            optional: false,
        })
    );
    // The Secret does not exist: the key reference and the optional `envFrom` both say so.
    assert_eq!(
        row(&rows, "TOKEN").value,
        EnvValue::Missing(Missing::Object {
            kind: SourceKind::Secret,
            name: Arc::from("web-secret"),
            optional: false,
        })
    );
    // An object nobody looked up reads as missing too, and keeps its `optional`.
    assert_eq!(
        row(&rows, "OPT").value,
        EnvValue::Missing(Missing::Object {
            kind: SourceKind::Secret,
            name: Arc::from("nobody"),
            optional: true,
        })
    );
    // One row stands for the keys of a source that is not there.
    let star = row(&rows, "*");
    assert_eq!(
        star.value,
        EnvValue::Missing(Missing::Object {
            kind: SourceKind::Secret,
            name: Arc::from("web-secret"),
            optional: true,
        })
    );
}

#[test]
fn an_unreadable_source_reports_why() {
    let sources = sources(
        None,
        Some(SourceState::Unavailable("forbidden: no access".into())),
    );
    let rows = rows_of(&env_pod(), &sources);
    assert!(matches!(
        &row(&rows, "TOKEN").value,
        EnvValue::Missing(Missing::Unavailable { reason, .. }) if reason.contains("forbidden")
    ));
}

#[test]
fn field_and_resource_refs_resolve_from_the_pod() {
    let rows = rows_of(&env_pod(), &sources(None, None));
    assert_eq!(
        row(&rows, "POD_IP").value,
        EnvValue::Plain("10.1.2.3".into())
    );
    assert_eq!(row(&rows, "TEAM").value, EnvValue::Plain("core".into()));
    assert_eq!(row(&rows, "APP").value, EnvValue::Plain("web".into()));
    assert_eq!(
        row(&rows, "IPS").value,
        EnvValue::Plain("10.1.2.3,fd00::3".into())
    );
    assert_eq!(row(&rows, "NOPE").value, EnvValue::Missing(Missing::Unset));
    assert_eq!(
        row(&rows, "CPU_LIMIT").value,
        EnvValue::Plain("500m".into())
    );
}

#[test]
fn source_refs_lists_each_object_once() {
    let detail = PodDetail::from_resource(&env_pod()).unwrap();
    assert_eq!(
        detail.source_refs(),
        vec![
            (SourceKind::ConfigMap, Arc::from("web-config")),
            (SourceKind::Secret, Arc::from("web-secret")),
            (SourceKind::Secret, Arc::from("nobody")),
        ]
    );
}

#[test]
fn mounts_probes_and_volumes() {
    let pod = pod(json!({
        "containers": [{
            "name": "app",
            "volumeMounts": [
                {"name": "cfg", "mountPath": "/etc/web", "readOnly": true},
                {"name": "data", "mountPath": "/data", "subPath": "db"},
            ],
            "livenessProbe": {"httpGet": {"path": "/healthz", "port": 8080, "scheme": "HTTP"},
                              "initialDelaySeconds": 10, "periodSeconds": 5},
            "readinessProbe": {"tcpSocket": {"port": "db"}},
            "startupProbe": {"exec": {"command": ["cat", "/tmp/ok"]}, "failureThreshold": 30},
        }],
        "volumes": [
            {"name": "cfg", "configMap": {"name": "web-config"}},
            {"name": "data", "persistentVolumeClaim": {"claimName": "web-data"}},
            {"name": "tmp", "emptyDir": {"medium": "Memory", "sizeLimit": "64Mi"}},
            {"name": "tok", "projected": {"sources": [{}, {}]}},
            {"name": "keys", "secret": {"secretName": "web-secret"}},
            {"name": "weird", "flexVolume": {"driver": "x"}},
        ],
    }));
    let detail = PodDetail::from_resource(&pod).unwrap();
    let app = &detail.containers[0];
    assert_eq!(app.mounts.len(), 2);
    assert_eq!(&*app.mounts[0].path, "/etc/web");
    assert_eq!(app.mounts[0].flags(), "read-only");
    assert_eq!(app.mounts[1].flags(), "subPath db");

    let kinds: Vec<_> = app.probes.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [
            ProbeKind::Liveness,
            ProbeKind::Readiness,
            ProbeKind::Startup
        ]
    );
    assert_eq!(app.probes[0].action, "http-get http://:8080/healthz");
    assert_eq!(
        app.probes[0].timing,
        "delay=10s timeout=1s period=5s #success=1 #failure=3"
    );
    assert_eq!(app.probes[1].action, "tcp-socket :db");
    assert_eq!(app.probes[2].action, "exec [cat /tmp/ok]");
    assert!(app.probes[2].timing.ends_with("#failure=30"));

    let volumes: Vec<(&str, &str, &str)> = detail
        .volumes
        .iter()
        .map(|v| (&*v.source, &*v.name, v.detail.as_str()))
        .collect();
    assert_eq!(
        volumes,
        [
            ("ConfigMap", "cfg", "web-config"),
            ("PersistentVolumeClaim", "data", "web-data"),
            ("EmptyDir", "tmp", "Memory, limit 64Mi"),
            ("Projected", "tok", "2 sources"),
            ("Secret", "keys", "web-secret"),
            ("FlexVolume", "weird", ""),
        ]
    );
}

#[test]
fn tolerations_affinity_node_and_owners() {
    let pod = pod(json!({
        "nodeName": "worker-1",
        "nodeSelector": {"disktype": "ssd"},
        "tolerations": [
            {"key": "node.kubernetes.io/not-ready", "operator": "Exists",
             "effect": "NoExecute", "tolerationSeconds": 300},
            {"operator": "Exists"},
        ],
        "affinity": {
            "nodeAffinity": {"requiredDuringSchedulingIgnoredDuringExecution": {
                "nodeSelectorTerms": [{"matchExpressions": [
                    {"key": "zone", "operator": "In", "values": ["a", "b"]}]}]}},
            "podAntiAffinity": {"preferredDuringSchedulingIgnoredDuringExecution": [{
                "weight": 50,
                "podAffinityTerm": {"topologyKey": "kubernetes.io/hostname",
                                    "labelSelector": {"matchLabels": {"app": "web"}}}}]},
        },
        "topologySpreadConstraints": [{"maxSkew": 2, "topologyKey": "zone",
                                       "whenUnsatisfiable": "ScheduleAnyway"}],
        "containers": [],
    }));
    let detail = PodDetail::from_resource(&pod).unwrap();
    assert_eq!(detail.node.as_deref(), Some("worker-1"));
    assert_eq!(
        detail.tolerations[0].display(),
        "node.kubernetes.io/not-ready:NoExecute op=Exists for 300s"
    );
    assert_eq!(detail.tolerations[1].display(), "(all taints) op=Exists");
    let lines: Vec<(String, String, String)> = detail
        .affinity
        .iter()
        .map(|l| (l.kind.to_owned(), l.strength.clone(), l.text.clone()))
        .collect();
    assert_eq!(
        lines,
        [
            ("Node selector".into(), "".into(), "disktype=ssd".into()),
            (
                "Node affinity".into(),
                "required".into(),
                "zone In [a,b]".into()
            ),
            (
                "Pod anti-affinity".into(),
                "preferred (weight 50)".into(),
                "app=web on kubernetes.io/hostname".into()
            ),
            (
                "Topology spread".into(),
                "ScheduleAnyway".into(),
                "maxSkew 2 over zone".into()
            ),
        ]
    );
    assert_eq!(detail.controller().map(|o| &*o.name), Some("web-5d"));
}

#[test]
fn a_pod_without_a_spec_degrades_and_another_kind_is_refused() {
    let detail = PodDetail::from_resource(&pod(json!({}))).unwrap();
    assert!(detail.containers.is_empty() && detail.volumes.is_empty() && detail.node.is_none());
    assert!(PodDetail::from_resource(&config_map(json!({}))).is_err());
}
