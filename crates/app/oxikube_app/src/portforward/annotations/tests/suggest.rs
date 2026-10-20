use std::net::SocketAddr;

use oxikube_domain::Resource;
use oxikube_testkit::resource;
use serde_json::json;

use crate::portforward::annotations::{AUTO_PORT_FORWARDS, PORT_FORWARDS, annotation_suggestions};
use crate::portforward::tests::{NS, pod_spec};
use crate::portforward::{ForwardId, ForwardInfo, ForwardState};

fn annotated(kind: &str, annotations: &[(&str, &str)]) -> Resource {
    let mut builder = resource("v1", kind).name("web").namespace(NS);
    for (key, value) in annotations {
        builder = builder.annotation(*key, *value);
    }
    builder.build()
}

fn active(local: u16) -> ForwardInfo {
    let addr: SocketAddr = ([127, 0, 0, 1], local).into();
    ForwardInfo {
        id: ForwardId(1),
        spec: pod_spec("db", 5432).with_name("db"),
        state: ForwardState::Active {
            pod: "db".into(),
            local_addr: addr,
        },
        local_addr: Some(addr),
        pod: Some("db".into()),
        last_error: None,
        stats: Default::default(),
        started_at: jiff::Timestamp::UNIX_EPOCH,
    }
}

#[test]
fn off_by_default_means_nothing() {
    let pod = annotated("Pod", &[(PORT_FORWARDS, "app::8080:80")]);
    assert!(annotation_suggestions(&pod, false, &[]).is_empty());
}

#[test]
fn on_returns_both_annotations_with_the_auto_flag() {
    let pod = annotated(
        "Pod",
        &[
            (PORT_FORWARDS, "app::8080:80, app::9090"),
            (AUTO_PORT_FORWARDS, "app::8080:80,app::3000:3000"),
        ],
    );
    let found = annotation_suggestions(&pod, true, &[]);
    let rows: Vec<_> = found.iter().map(|s| (s.label(), s.forward.auto)).collect();
    assert_eq!(
        rows,
        vec![
            ("app 8080:80".to_owned(), true),
            ("app 9090:9090".to_owned(), false),
            ("app 3000:3000".to_owned(), true),
        ]
    );
}

#[test]
fn a_plain_pod_or_another_kind_has_none() {
    assert!(annotation_suggestions(&annotated("Pod", &[]), true, &[]).is_empty());
    let svc = annotated("Service", &[(PORT_FORWARDS, "8080:80")]);
    assert!(annotation_suggestions(&svc, true, &[]).is_empty());
    let junk = annotated("Pod", &[(PORT_FORWARDS, "not a forward")]);
    assert!(annotation_suggestions(&junk, true, &[]).is_empty());
    let _ = json!(null);
}

#[test]
fn a_taken_local_port_warns() {
    let pod = annotated("Pod", &[(PORT_FORWARDS, "8080:80,8081:81")]);
    let found = annotation_suggestions(&pod, true, &[active(8080)]);
    assert!(
        found[0]
            .conflict
            .as_deref()
            .is_some_and(|c| c.contains("8080") && c.contains("db"))
    );
    assert_eq!(found[1].conflict, None);
}
