//! What the target must have: the kind, the apiVersion, the namespace; and the references that
//! are only warned about.

use oxikube_domain::kinds::ResourceKind;
use oxikube_testkit::kinds::kind;

use super::*;

fn without(kinds: Vec<ResourceKind>, name: &str) -> Vec<ResourceKind> {
    kinds.into_iter().filter(|k| &*k.gvk.kind != name).collect()
}

#[test]
fn a_kind_the_target_does_not_serve_is_refused_with_nothing_written() {
    let t = Copying::new(false);
    let target = t.h.connector.ports_for(&id("b")).discovery;
    target.set_kinds(without(oxikube_testkit::kinds::core_kinds(), "Deployment"));
    let error = t.preview(t.request(&deployment(), "prod")).unwrap_err();
    let CopyError::KindNotServed(message) = &error else {
        panic!("kind not served, got {error:?}")
    };
    assert!(message.contains("Deployment"), "{message}");
    assert!(t.h.writes("b").is_empty());
}

#[test]
fn an_apiversion_the_target_does_not_serve_names_the_ones_it_does() {
    let t = Copying::new(false);
    let mut kinds = without(oxikube_testkit::kinds::core_kinds(), "Deployment");
    kinds.push(kind("apps", "v1beta2", "Deployment", "deployments").build());
    t.h.connector.ports_for(&id("b")).discovery.set_kinds(kinds);
    let error = t.preview(t.request(&deployment(), "prod")).unwrap_err();
    let CopyError::VersionNotServed(message) = &error else {
        panic!("version not served, got {error:?}")
    };
    assert!(message.contains("apps/v1beta2"), "{message}");
    assert!(message.contains("apps/v1"), "{message}");
}

#[test]
fn a_missing_namespace_is_refused_and_creating_it_is_a_separate_guarded_step() {
    let t = Copying::new(false);
    let request = t.request(&configmap(), "brand-new");
    let error = t.preview(request.clone()).unwrap_err();
    assert_eq!(error, CopyError::NamespaceMissing("brand-new".into()));
    assert!(t.h.writes("b").is_empty());

    t.service
        .create_namespace(&id("b"), "brand-new")
        .now_or_never()
        .expect("answers at once")
        .expect("create");
    let created: Vec<_> =
        t.h.writes("b")
            .into_iter()
            .filter(|call| !call.is_dry_run())
            .collect();
    assert_eq!(created.len(), 1);
    assert!(
        t.h.audit()
            .iter()
            .any(|r| r.cmd.as_ref() == "namespace::Create")
    );
    t.preview(request).expect("the namespace is there now");
}

#[test]
fn the_namespaces_of_a_cluster_are_listed_sorted() {
    let t = Copying::new(false);
    let names = t
        .service
        .namespaces(&id("b"))
        .now_or_never()
        .expect("answers at once")
        .expect("list");
    assert_eq!(names, ["prod", "staging"]);
}

#[test]
fn references_the_target_lacks_are_warnings_not_errors() {
    let t = Copying::new(false);
    let workload = json!({
        "apiVersion": "apps/v1", "kind": "Deployment",
        "metadata": {"name": "web", "namespace": "shop"},
        "spec": {"selector": {"matchLabels": {"app": "web"}}, "template": {
            "metadata": {"labels": {"app": "web"}},
            "spec": {
                "serviceAccountName": "web-sa",
                "imagePullSecrets": [{"name": "registry"}],
                "containers": [{"name": "web", "image": "nginx",
                    "envFrom": [{"configMapRef": {"name": "settings"}}],
                    "env": [{"name": "TOKEN", "valueFrom": {"secretKeyRef": {"name": "token", "key": "t"}}}]}],
                "volumes": [{"name": "data", "persistentVolumeClaim": {"claimName": "data"}}]
            }}}
    });
    // The target has the ConfigMap only.
    t.target().insert(
        Resource::from_json(json!({"apiVersion": "v1", "kind": "ConfigMap",
        "metadata": {"name": "settings", "namespace": "prod"}}))
        .unwrap(),
    );
    let preview = t.preview(t.request(&workload, "prod")).expect("preview");
    let mut missing: Vec<String> = preview.missing.iter().map(|m| m.message()).collect();
    missing.sort();
    assert_eq!(
        missing,
        [
            "PersistentVolumeClaim data is not in namespace prod on the target",
            "Secret registry is not in namespace prod on the target",
            "Secret token is not in namespace prod on the target",
            "ServiceAccount web-sa is not in namespace prod on the target",
        ]
    );
    // Only a warning: the copy goes ahead.
    t.copy(&preview, None).expect("copy");
    assert_eq!(t.target_writes().len(), 1);
}

#[test]
fn a_cronjob_and_a_bare_pod_are_scanned_too() {
    let t = Copying::new(false);
    let pod = json!({"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p", "namespace": "shop"},
        "spec": {"containers": [{"name": "c", "image": "x",
            "envFrom": [{"secretRef": {"name": "creds"}}]}]}});
    let preview = t.preview(t.request(&pod, "prod")).expect("preview");
    assert_eq!(preview.missing.len(), 1);
    assert_eq!(preview.missing[0].kind, "Secret");
    let cron = json!({"apiVersion": "batch/v1", "kind": "CronJob",
        "metadata": {"name": "nightly", "namespace": "shop"},
        "spec": {"schedule": "0 * * * *", "jobTemplate": {"spec": {"template": {"spec": {
            "containers": [{"name": "c", "image": "x"}],
            "volumes": [{"name": "v", "configMap": {"name": "cron-config"}}]}}}}}});
    let preview = t.preview(t.request(&cron, "prod")).expect("preview");
    assert_eq!(preview.missing.len(), 1);
    assert_eq!(preview.missing[0].name, "cron-config");
}

#[test]
fn an_object_that_cannot_be_read_from_the_source_is_reported() {
    let t = Copying::new(false);
    let request = CopyRequest::to(source_ref(&configmap()), id("b"), Some("prod"));
    let error = t.preview(request).unwrap_err();
    assert!(
        matches!(error, CopyError::SourceUnavailable(_)),
        "{error:?}"
    );
    assert!(t.h.writes("b").is_empty());
}

#[test]
fn a_target_that_is_not_connected_is_connected_on_demand() {
    let h = Harness::with_extra(|registry, _, sessions| {
        register_commands(registry, sessions.clone())?;
        crate::namespaces::register_commands(registry)
    });
    // Only the source is connected; `b` is in the catalog (the fake source) and nowhere else.
    h.connect("a", false);
    for name in ["a", "b"] {
        h.connector
            .ports_for(&id(name))
            .discovery
            .set_kinds(oxikube_testkit::kinds::core_kinds());
    }
    h.resources("b").insert(namespace_object("prod"));
    h.resources("a")
        .insert(Resource::from_json(configmap()).unwrap());
    assert!(h.manager.get(&id("b")).is_none());
    let service = CopyService::new(h.bus.clone(), h.manager.clone(), "alice");
    let request = CopyRequest::to(source_ref(&configmap()), id("b"), Some("prod"));
    let preview = service
        .preview(request)
        .now_or_never()
        .expect("the fakes answer at once")
        .expect("preview");
    assert!(h.manager.get(&id("b")).is_some_and(|s| s.is_connected()));
    assert_eq!(preview.target.namespace(), Some("prod"));
    // And a context the catalog does not have cannot be connected.
    let nowhere = CopyRequest::to(source_ref(&configmap()), id("nowhere"), Some("prod"));
    let error = service
        .preview(nowhere)
        .now_or_never()
        .unwrap()
        .unwrap_err();
    assert!(matches!(error, CopyError::NotConnected(_)), "{error:?}");
}

#[test]
fn the_clusters_to_pick_list_the_connected_ones_with_their_read_only_flag() {
    let t = Copying::new(true);
    let choices = t
        .service
        .clusters()
        .now_or_never()
        .expect("answers at once");
    let summary: Vec<(String, bool, bool)> = choices
        .iter()
        .map(|c| (c.name.clone(), c.connected, c.read_only))
        .collect();
    assert_eq!(
        summary,
        [("a".to_owned(), true, false), ("b".to_owned(), true, true)]
    );
}
