//! One case per kind: fixture (or inline manifest) in, expected cell texts out. Together with the
//! pod tests this covers all ~40 kinds of the catalogue.

use oxikube_testkit::fixtures as fx;
use serde_json::json;

use super::{check, cluster, namespaced};

#[test]
fn deployment() {
    check(
        &fx::deployment_ready(),
        &[
            ("name", "web"),
            ("namespace", "demo"),
            ("ready", "3/3"),
            ("up-to-date", "3"),
            ("available", "3"),
            ("age", "27h"),
            ("images", "registry.example/app:1.0"),
            ("selector", "app=web"),
        ],
    );
    check(
        &fx::deployment_progressing(),
        &[("ready", "2/3"), ("up-to-date", "3"), ("available", "2")],
    );
}

#[test]
fn replica_sets_stateful_sets_and_daemon_sets() {
    check(
        &fx::replicaset(),
        &[
            ("desired", "3"),
            ("current", "3"),
            ("ready", "3"),
            ("controlled-by", "Deployment/web"),
        ],
    );
    check(
        &fx::statefulset(),
        &[("ready", "3/3"), ("images", "registry.example/app:1.0")],
    );
    check(
        &fx::daemonset(),
        &[
            ("desired", "2"),
            ("current", "2"),
            ("ready", "1"),
            ("up-to-date", "2"),
            ("available", "1"),
        ],
    );
}

#[test]
fn jobs_and_cron_jobs() {
    check(
        &fx::job_complete(),
        &[
            ("status", "Complete"),
            ("completions", "3/3"),
            ("duration", "59m"),
        ],
    );
    check(
        &fx::job_failed(),
        &[("status", "Failed"), ("completions", "0/1")],
    );
    check(
        &fx::cronjob(),
        &[
            ("schedule", "0 3 * * *"),
            ("suspend", "False"),
            ("active", "1"),
            ("last-schedule", "24h"),
            ("timezone", "Etc/UTC"),
        ],
    );
}

#[test]
fn replication_controller() {
    let rc = namespaced(
        "v1",
        "ReplicationController",
        json!({"spec": {"replicas": 3, "selector": {"app": "web"}}, "status": {"replicas": 3, "readyReplicas": 2}}),
    );
    check(
        &rc,
        &[
            ("desired", "3"),
            ("current", "3"),
            ("ready", "2"),
            ("selector", "app=web"),
        ],
    );
}

#[test]
fn nodes() {
    check(
        &fx::node_control_plane(),
        &[
            ("name", "oxikube-control-plane"),
            ("status", "Ready"),
            ("roles", "control-plane"),
            ("version", "v1.33.1"),
            ("internal-ip", "172.18.0.3"),
            ("taints", "1"),
            ("os-image", "Debian GNU/Linux 12 (bookworm)"),
            ("arch", "amd64"),
            ("allocatable-cpu", "4"),
            ("allocatable-memory", "16Gi"),
        ],
    );
    check(&fx::node_ready(), &[("roles", "<none>")]);
    check(&fx::node_not_ready(), &[("status", "NotReady")]);
    check(
        &fx::node_cordoned(),
        &[("status", "Ready,SchedulingDisabled")],
    );
}

#[test]
fn namespace_and_service_account() {
    check(&fx::namespace(), &[("name", "demo"), ("status", "Active")]);
    let sa = namespaced(
        "v1",
        "ServiceAccount",
        json!({"secrets": [{"name": "a"}, {"name": "b"}]}),
    );
    check(&sa, &[("secrets", "2")]);
}

#[test]
fn services_and_endpoints() {
    check(
        &fx::service(),
        &[
            ("type", "ClusterIP"),
            ("cluster-ip", "10.96.12.34"),
            ("external-ip", ""),
            ("ports", "80/TCP"),
            ("selector", "app=web"),
        ],
    );
    let lb = namespaced(
        "v1",
        "Service",
        json!({
            "spec": {"type": "LoadBalancer", "clusterIP": "10.0.0.1",
                     "ports": [{"port": 80, "nodePort": 30080, "protocol": "TCP"}, {"port": 53, "protocol": "UDP"}]},
            "status": {"loadBalancer": {"ingress": [{"ip": "203.0.113.5"}, {"hostname": "lb.example.com"}]}}
        }),
    );
    check(
        &lb,
        &[
            ("ports", "80:30080/TCP,53/UDP"),
            ("external-ip", "203.0.113.5,lb.example.com"),
        ],
    );
    let external_name = namespaced(
        "v1",
        "Service",
        json!({"spec": {"type": "ExternalName", "externalName": "db.example.com"}}),
    );
    check(&external_name, &[("external-ip", "db.example.com")]);

    let endpoints = namespaced(
        "v1",
        "Endpoints",
        json!({"subsets": [{"addresses": [{"ip": "10.0.0.1"}, {"ip": "10.0.0.2"}, {"ip": "10.0.0.3"}, {"ip": "10.0.0.4"}], "ports": [{"port": 8080}]}]}),
    );
    check(
        &endpoints,
        &[(
            "endpoints",
            "10.0.0.1:8080,10.0.0.2:8080,10.0.0.3:8080 + 1 more...",
        )],
    );

    let slice = namespaced(
        "discovery.k8s.io/v1",
        "EndpointSlice",
        json!({"addressType": "IPv4", "ports": [{"port": 80}, {"port": 443}], "endpoints": [{"addresses": ["10.0.0.1"]}, {"addresses": ["10.0.0.2"]}]}),
    );
    check(
        &slice,
        &[
            ("address-type", "IPv4"),
            ("ports", "80,443"),
            ("endpoints", "10.0.0.1,10.0.0.2"),
        ],
    );
}

#[test]
fn ingress_ingress_class_and_network_policy() {
    check(
        &fx::ingress(),
        &[
            ("class", "nginx"),
            ("hosts", "web.example.com,*"),
            ("address", "203.0.113.10"),
            ("ports", "80, 443"),
        ],
    );
    let class = cluster(
        "networking.k8s.io/v1",
        "IngressClass",
        json!({"spec": {"controller": "k8s.io/ingress-nginx", "parameters": {"kind": "IngressParameters", "name": "p"}}}),
    );
    check(
        &class,
        &[
            ("controller", "k8s.io/ingress-nginx"),
            ("parameters-kind", "IngressParameters"),
            ("parameters-name", "p"),
        ],
    );
    let policy = namespaced(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        json!({"spec": {"podSelector": {"matchLabels": {"app": "db"}}, "policyTypes": ["Ingress", "Egress"]}}),
    );
    check(
        &policy,
        &[
            ("pod-selector", "app=db"),
            ("policy-types", "Ingress,Egress"),
        ],
    );
    let default_types = namespaced(
        "networking.k8s.io/v1",
        "NetworkPolicy",
        json!({"spec": {"podSelector": {}}}),
    );
    check(&default_types, &[("policy-types", "Ingress")]);
}
