//! `ingressclass::SetDefault` through the real `CommandBus` and `MutationGuard` over the testkit
//! fakes (see [`crate::testing::Harness`]): the patch it sends, the previous default it clears,
//! read-only mode, the dry run, the audit and the tool stub.

use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::{ConfirmTier, Risk};
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::{Patch, PatchKind};
use oxikube_testkit::ResourceCall;
use serde_json::{Value, json};

use crate::command_bus::{DispatchError, Outcome};
use crate::testing::{Harness, ctx, id};

const ANNOTATION: &str = "ingressclass.kubernetes.io/is-default-class";

fn gvk() -> Gvk {
    Gvk::new("networking.k8s.io", "v1", "IngressClass")
}

fn class(name: &str, default: bool) -> Resource {
    let annotations = if default {
        json!({ANNOTATION: "true"})
    } else {
        json!({})
    };
    Resource::from_json(json!({
        "apiVersion": "networking.k8s.io/v1", "kind": "IngressClass",
        "metadata": {"name": name, "annotations": annotations},
        "spec": {"controller": "k8s.io/ingress-nginx"}
    }))
    .unwrap()
}

fn set_default(cluster: &str, name: &str, clear_previous: bool) -> Command {
    Command::IngressClassSetDefault {
        target: ResourceRef::cluster_scoped(id(cluster), gvk(), name),
        clear_previous,
    }
}

fn harness_with(classes: impl IntoIterator<Item = Resource>) -> Harness {
    let h = Harness::with_ingress_class_handler();
    h.connect("a", false);
    for class in classes {
        h.resources("a").insert(class);
    }
    h
}

/// The merge patches the cluster received: (class, annotation value sent, was a dry run).
fn patches(h: &Harness) -> Vec<(String, Value, bool)> {
    h.writes("a")
        .into_iter()
        .map(|call| match call {
            ResourceCall::Patch {
                kind,
                namespace,
                name,
                patch: Patch { kind: patch, body },
                options,
            } => {
                assert_eq!(kind, gvk());
                assert_eq!(namespace, None, "IngressClass is cluster-scoped");
                assert_eq!(patch, PatchKind::Merge);
                let value = body["metadata"]["annotations"][ANNOTATION].clone();
                assert_eq!(
                    body["metadata"]["annotations"].as_object().map(|o| o.len()),
                    Some(1),
                    "only the default annotation is touched"
                );
                (name, value, options.dry_run)
            }
            other => panic!("unexpected write {other:?}"),
        })
        .collect()
}

/// The patches that were not dry runs: what the confirmed command changed.
fn sent(h: &Harness) -> Vec<(String, Value, bool)> {
    patches(h)
        .into_iter()
        .filter(|(_, _, dry_run)| !dry_run)
        .collect()
}

fn script_patches(h: &Harness, names: &[&str]) {
    for name in names {
        h.resources("a").script().patch.push_ok(class(name, true));
    }
}

#[test]
fn set_default_annotates_the_class_after_a_low_risk_confirmation() {
    let h = harness_with([class("nginx", false), class("traefik", false)]);
    script_patches(&h, &["nginx"]);
    let cmd = set_default("a", "nginx", true);

    let request = h.ask(cmd.clone(), ctx(Initiator::Ui));
    assert_eq!(request.tier, ConfirmTier::Simple);
    assert_eq!(request.risk, Some(Risk::Low));
    assert_eq!(request.expected_name, None);
    assert!(
        request
            .summary
            .starts_with("Set Default Ingress Class: IngressClass nginx on a"),
        "{}",
        request.summary
    );
    // Only the guard's dry run is sent before the answer (E19-S07), and the question carries
    // the class as the server would store it.
    assert!(
        sent(&h).is_empty() && !patches(&h).is_empty(),
        "nothing is sent before the answer"
    );
    let Some(crate::guard::DryRunPreview::Rehearsed(preview)) = &request.preview else {
        panic!("a patch is previewed: {request:?}");
    };
    assert_eq!(&*preview.changes[0].target.name, "nginx");
    assert_eq!(preview.changes[0].change, crate::apply::Change::Update);

    let answer = crate::guard::Confirmation::simple(request.token);
    let out = h
        .dispatch(cmd, ctx(Initiator::Ui).with_confirmation(answer))
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some("IngressClass nginx is the default Ingress class")
    );
    assert_eq!(sent(&h), [("nginx".to_owned(), json!("true"), false)]);
    let audit = h.audit();
    assert_eq!(
        h.outcomes(),
        [
            (AuditOutcome::Succeeded, Initiator::Ui),
            (AuditOutcome::Succeeded, Initiator::Ui)
        ],
        "the guard's dry run, then the patch"
    );
    assert!(audit[0].dry_run && !audit[1].dry_run);
    assert_eq!(&*audit[1].cmd, "ingressclass::SetDefault");
    assert_eq!(audit[1].target.name.as_ref(), "nginx");
}

#[test]
fn the_class_that_was_the_default_loses_the_flag() {
    let h = harness_with([
        class("nginx", false),
        class("traefik", true),
        class("haproxy", true),
    ]);
    script_patches(&h, &["nginx", "haproxy", "traefik"]);
    let out = h
        .confirm_and_run(set_default("a", "nginx", true), ctx(Initiator::Ui))
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    // The new default first, so there is never a moment without one; then the old ones, whose
    // annotation a null merge-patch value removes.
    let sent = sent(&h);
    assert_eq!(sent[0], ("nginx".to_owned(), json!("true"), false));
    let mut cleared: Vec<_> = sent[1..].to_vec();
    cleared.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        cleared,
        [
            ("haproxy".to_owned(), Value::Null, false),
            ("traefik".to_owned(), Value::Null, false)
        ]
    );
    let data = output.data.unwrap();
    assert_eq!(data["outcome"], "default_set");
    assert_eq!(data["cleared"].as_array().map(Vec::len), Some(2));
}

#[test]
fn the_summary_says_the_others_are_cleared_only_when_they_are() {
    let h = harness_with([class("nginx", false)]);
    let with = h.ask(set_default("a", "nginx", true), ctx(Initiator::Ui));
    assert!(
        with.summary
            .ends_with("Clears the default flag of the other Ingress classes.")
    );
    let without = h.ask(set_default("a", "nginx", false), ctx(Initiator::Ui));
    assert!(!without.summary.contains("Clears"), "{}", without.summary);
}

#[test]
fn without_clear_previous_only_the_target_is_patched_and_nothing_is_listed() {
    let h = harness_with([class("nginx", false), class("traefik", true)]);
    script_patches(&h, &["nginx"]);
    h.confirm_and_run(set_default("a", "nginx", false), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(sent(&h).len(), 1);
    let listed = h
        .resources("a")
        .recorded_calls()
        .iter()
        .any(|call| matches!(call, ResourceCall::List { .. }));
    assert!(!listed);
}

#[test]
fn the_class_that_is_already_the_default_is_not_cleared_from_itself() {
    let h = harness_with([class("nginx", true)]);
    script_patches(&h, &["nginx"]);
    h.confirm_and_run(set_default("a", "nginx", true), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(sent(&h), [("nginx".to_owned(), json!("true"), false)]);
}

#[test]
fn a_read_only_cluster_refuses_every_initiator_before_any_request() {
    let h = Harness::with_ingress_class_handler();
    h.connect("a", true);
    h.resources("a").insert(class("nginx", false));
    for initiator in Initiator::ALL {
        let err = h
            .dispatch(set_default("a", "nginx", true), ctx(initiator))
            .unwrap_err();
        assert!(matches!(err, DispatchError::ReadOnly { .. }), "{initiator}");
    }
    assert!(h.writes("a").is_empty());
    assert_eq!(h.audit().len(), Initiator::ALL.len());
    assert!(h.audit().iter().all(|r| r.outcome == AuditOutcome::Denied));
}

#[test]
fn a_dry_run_dispatch_rehearses_every_patch_on_the_server() {
    let h = harness_with([class("nginx", false), class("traefik", true)]);
    script_patches(&h, &["nginx", "traefik"]);
    let out = h
        .confirm_and_run(
            set_default("a", "nginx", true),
            ctx(Initiator::Command).with_dry_run(true),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("expected completion");
    };
    assert_eq!(
        output.message.as_deref(),
        Some("Dry run: IngressClass nginx can be made the default")
    );
    assert!(patches(&h).iter().all(|(_, _, dry_run)| *dry_run));
    assert_eq!(patches(&h).len(), 2);
    assert!(h.audit()[0].dry_run);
}

#[test]
fn a_refused_patch_is_an_error_and_clears_nothing() {
    let h = harness_with([class("nginx", false), class("traefik", true)]);
    h.resources("a")
        .script()
        .patch
        .push_err(OxiError::forbidden("ingressclasses nginx is forbidden"));
    // The server refuses the guard's dry run: the command is refused before anything is asked.
    let err = h
        .dispatch(set_default("a", "nginx", true), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err:?}");
    assert_eq!(patches(&h), [("nginx".to_owned(), json!("true"), true)]);
    assert_eq!(h.outcomes(), [(AuditOutcome::Failed, Initiator::Ui)]);
    assert!(h.audit()[0].dry_run);
}

#[test]
fn a_flag_that_cannot_be_cleared_is_reported_with_the_class_that_is_now_default() {
    let h = harness_with([class("nginx", false), class("traefik", true)]);
    // The guard's dry run passes; then the new default is set and the old one cannot be cleared.
    script_patches(&h, &["nginx", "traefik", "nginx"]);
    h.resources("a")
        .script()
        .patch
        .push_err(OxiError::conflict("the object has been modified"));
    let err = h
        .confirm_and_run(set_default("a", "nginx", true), ctx(Initiator::Ui))
        .unwrap_err();
    let DispatchError::Handler(error) = err else {
        panic!("expected a handler error, got {err:?}");
    };
    assert!(
        error.message().contains("nginx is now the default"),
        "{error}"
    );
    assert!(error.message().contains("traefik"), "{error}");
}

#[test]
fn only_an_ingress_class_is_accepted() {
    let h = harness_with([]);
    let cmd = Command::IngressClassSetDefault {
        target: ResourceRef::namespaced(id("a"), Gvk::new("", "v1", "ConfigMap"), "default", "c"),
        clear_previous: true,
    };
    let err = h.confirm_and_run(cmd, ctx(Initiator::Ui)).unwrap_err();
    assert!(
        matches!(err, DispatchError::Handler(e) if e.kind() == oxikube_domain::ErrorKind::Validation)
    );
    assert!(h.writes("a").is_empty());
}

#[test]
fn the_command_has_a_tool_stub_and_a_palette_entry_for_ingress_classes_only() {
    let h = harness_with([]);
    let tool = h
        .bus
        .tool(CommandId::INGRESS_CLASS_SET_DEFAULT)
        .expect("a stub tool");
    assert_eq!(tool.name.to_string(), "k8s.ingressclass_set_default");
    let meta = oxikube_domain::command::lookup(CommandId::INGRESS_CLASS_SET_DEFAULT).unwrap();
    assert!(meta.mutating && meta.availability.requires_writable);
    assert_eq!(meta.risk, Some(Risk::Low));
}

#[test]
fn the_palette_lists_it_for_an_ingress_class_only_and_not_on_a_read_only_cluster() {
    use oxikube_domain::Capabilities;
    use oxikube_domain::command::ViewContext;

    use crate::command_bus::{CommandContext, Selection};

    let h = harness_with([]);
    let table = || {
        let mut ctx =
            CommandContext::new(ViewContext::Table).with_capabilities(Capabilities::all());
        ctx.cluster_active = true;
        ctx
    };
    let listed = |ctx: &CommandContext| {
        h.bus
            .list(ctx)
            .iter()
            .any(|info| info.id() == CommandId::INGRESS_CLASS_SET_DEFAULT)
    };
    let class = table().selecting(Selection::one(gvk()));
    assert!(listed(&class), "an IngressClass is selected");
    assert!(!listed(&table()), "nothing selected");
    assert!(
        !listed(&table().selecting(Selection::one(Gvk::new("", "v1", "Pod")))),
        "a Pod is selected"
    );
    let mut read_only = class;
    read_only.read_only = true;
    assert!(
        !listed(&read_only),
        "a read-only session is never offered a mutation"
    );
}
