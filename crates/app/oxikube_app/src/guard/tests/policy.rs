//! The confirmation tier per `CommandMeta` / `Risk` matrix, and the pure helpers.

use oxikube_domain::Capabilities;
use oxikube_domain::command::{COMMANDS, Command, CommandId, CommandMeta, CommandScope};
use oxikube_domain::safety::{ConfirmTier, Risk};

use crate::guard::policy;
use crate::testing::{id, node, pod, pod_delete};

const TEST_ID: CommandId = CommandId::new("test::Mutate");

fn mutation(risk: Risk) -> CommandMeta {
    CommandMeta::mutation(
        TEST_ID,
        "Mutate",
        CommandScope::Selection,
        risk,
        Capabilities::empty(),
    )
}

#[test]
fn tier_follows_the_risk_matrix() {
    let expected = [
        (Risk::Low, ConfirmTier::Simple),
        (Risk::Medium, ConfirmTier::Simple),
        (Risk::High, ConfirmTier::TypeName),
        (Risk::Irreversible, ConfirmTier::TypeName),
    ];
    for (risk, tier) in expected {
        assert_eq!(policy::confirm_tier(&mutation(risk)), tier, "{risk}");
    }
}

#[test]
fn reads_and_privileged_commands_never_confirm() {
    let read = CommandMeta::read(TEST_ID, "Read", CommandScope::Global, Capabilities::empty());
    assert_eq!(policy::confirm_tier(&read), ConfirmTier::None);
    let privileged = CommandMeta::privileged(
        TEST_ID,
        "Lift",
        CommandScope::Cluster,
        Capabilities::empty(),
    );
    assert_eq!(policy::confirm_tier(&privileged), ConfirmTier::None);
}

#[test]
fn metadata_can_raise_the_tier_but_never_lower_it() {
    let raised = CommandMeta {
        confirm: ConfirmTier::TypeName,
        ..mutation(Risk::Low)
    };
    assert_eq!(policy::confirm_tier(&raised), ConfirmTier::TypeName);

    let lowered = CommandMeta {
        confirm: ConfirmTier::None,
        ..mutation(Risk::High)
    };
    assert_eq!(policy::confirm_tier(&lowered), ConfirmTier::TypeName);

    let no_risk = CommandMeta {
        confirm: ConfirmTier::None,
        risk: None,
        ..mutation(Risk::Low)
    };
    assert_eq!(
        policy::confirm_tier(&no_risk),
        ConfirmTier::TypeName,
        "a mutation without a declared risk fails safe"
    );
}

#[test]
fn every_declared_command_gets_its_declared_tier() {
    for meta in COMMANDS {
        let tier = policy::confirm_tier(meta);
        assert_eq!(tier, meta.confirm, "{}", meta.id);
        assert_eq!(tier != ConfirmTier::None, meta.mutating, "{}", meta.id);
    }
}

#[test]
fn cluster_and_audit_target_come_from_the_payload() {
    let cmd = pod_delete("a", "web-0");
    assert_eq!(policy::cluster_of(&cmd), Some(&id("a")));
    assert_eq!(policy::audit_target(&cmd, &id("a")), pod("a", "web-0"));

    let apply = Command::ResourceApply {
        cluster: id("b"),
        namespace: None,
        manifest: "kind: Secret\ndata:\n  token: c2VjcmV0\n".into(),
        field_manager: None,
        force: false,
        resource_version: None,
        reviewed_absent: false,
    };
    assert_eq!(policy::cluster_of(&apply), Some(&id("b")));
    let target = policy::audit_target(&apply, &id("b"));
    assert_eq!(target.cluster, id("b"));
    assert_eq!(target.gvk.kind.as_ref(), "Cluster");
    assert_eq!(target.name.as_ref(), "*");

    assert_eq!(policy::cluster_of(&Command::PaletteToggle), None);
}

#[test]
fn summary_and_expected_name_name_the_target_and_cluster() {
    let meta = pod_delete("a", "x").meta();
    assert_eq!(
        policy::summary(meta, &pod_delete("a", "web-0"), "prod"),
        "Delete Pod: Pod default/web-0 on prod"
    );
    let drain = Command::NodeDrain {
        target: node("a", "worker-1"),
        flags: oxikube_domain::drain::DrainFlags {
            force: true,
            ..Default::default()
        },
    };
    assert_eq!(
        policy::summary(drain.meta(), &drain, "prod"),
        "Drain Node: Node worker-1 on prod"
    );
    assert_eq!(policy::expected_name(&drain, "prod"), "worker-1");
    let apply = Command::ResourceApply {
        cluster: id("a"),
        namespace: None,
        manifest: String::new(),
        field_manager: None,
        force: false,
        resource_version: None,
        reviewed_absent: false,
    };
    assert_eq!(policy::expected_name(&apply, "prod"), "prod");
    assert_eq!(
        policy::summary(apply.meta(), &apply, "prod"),
        "Apply Manifest on prod"
    );
}

mod properties {
    //! The policy is a pure function of the command: monotonic in risk, never `None` for a
    //! mutation, and never below `TypeName` for the kinds the glossary says need a typed name.

    use oxikube_domain::command::{Command, Propagation};
    use oxikube_domain::ids::{Gvk, ResourceRef};
    use oxikube_domain::safety::{ConfirmTier, Risk};
    use proptest::prelude::*;

    use super::mutation;
    use crate::guard::policy;
    use crate::testing::id;

    fn risk() -> impl Strategy<Value = Risk> {
        prop::sample::select(Risk::ALL.to_vec())
    }

    fn tier() -> impl Strategy<Value = ConfirmTier> {
        prop::sample::select(ConfirmTier::ALL.to_vec())
    }

    /// A delete of a core or custom kind, namespaced or not, with any propagation.
    fn delete() -> impl Strategy<Value = Command> {
        let kinds = prop::sample::select(vec![
            ("", "Namespace"),
            ("", "PersistentVolume"),
            ("", "Node"),
            ("apiextensions.k8s.io", "CustomResourceDefinition"),
            ("", "Pod"),
            ("apps", "Deployment"),
            ("", "ConfigMap"),
            ("example.com", "Namespace"),
            ("example.com", "Widget"),
        ]);
        let propagation = prop::sample::select(vec![
            Propagation::Background,
            Propagation::Foreground,
            Propagation::Orphan,
        ]);
        (kinds, propagation, any::<bool>(), "[a-z][a-z0-9-]{0,20}").prop_map(
            |((group, kind), propagation, namespaced, name)| {
                let gvk = Gvk::new(group, "v1", kind);
                let target = if namespaced {
                    ResourceRef::namespaced(id("a"), gvk, "default", name.as_str())
                } else {
                    ResourceRef::cluster_scoped(id("a"), gvk, name.as_str())
                };
                Command::ResourceDelete {
                    target,
                    propagation,
                }
            },
        )
    }

    proptest! {
        #[test]
        fn the_tier_is_monotonic_in_risk(a in risk(), b in risk(), declared in tier()) {
            let (low, high) = (a.min(b), a.max(b));
            let tier_of = |risk| policy::confirm_tier(&oxikube_domain::command::CommandMeta {
                confirm: declared,
                ..mutation(risk)
            });
            prop_assert!(tier_of(low) <= tier_of(high));
            prop_assert!(tier_of(low) >= declared, "metadata never lowers the tier");
            prop_assert_ne!(tier_of(low), ConfirmTier::None, "a mutation always confirms");
            prop_assert_eq!(tier_of(low) == ConfirmTier::TypeName, low >= Risk::High || declared == ConfirmTier::TypeName);
        }

        #[test]
        fn a_delete_takes_a_typed_name_whenever_its_target_makes_it_high(command in delete()) {
            let tier = policy::confirm_tier_for(command.meta(), &command);
            let risk = command.effective_risk().expect("a delete has a risk");
            prop_assert!(tier >= policy::confirm_tier(command.meta()));
            prop_assert_eq!(tier, risk.confirm_tier().max(command.meta().confirm));
            let Command::ResourceDelete { target, propagation } = &command else {
                unreachable!("the strategy builds deletes");
            };
            let core = target.gvk.group.is_empty();
            let glossary_type_name = (core
                && matches!(&*target.gvk.kind, "Namespace" | "PersistentVolume" | "Node"))
                || &*target.gvk.kind == "CustomResourceDefinition"
                || *propagation == Propagation::Foreground;
            if glossary_type_name {
                prop_assert_eq!(tier, ConfirmTier::TypeName, "{:?}", target);
            }
        }
    }
}
