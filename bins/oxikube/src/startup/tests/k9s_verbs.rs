//! The k9s verbs (epic E12 criterion 2, #397: each action story binds its own verb, E12-S20
//! asserts them): in the shipped default keymap, on the real init order, each verb resolves to its
//! action in the table of every kind whose rows offer the verb's command
//! (`oxikube_testkit::sections`), and a kind-scoped verb resolves to nothing of the sort on the
//! kinds that do not offer it.
//!
//! `s` scale / shell, `r` restart / drain, `t` trigger, `z` sanitize, `u` used-by, `x` decode, `i`
//! set image, `ctrl-l` rollback, `c` / `u` cordon / uncordon; and the kind keys that share a
//! letter with them (`s` suspend on Jobs and CronJobs, `u` use namespace, `u` a Service's pods).

use gpui::TestAppContext;
use oxikube_domain::command::CommandId;
use oxikube_keymap::{parse_stack, resolve};
use oxikube_testkit::sections::{KindCase, catalog};

use crate::startup::{StartupEnv, init};

/// A verb: the command it runs, its key, the action the key names, and whether the binding is
/// scoped to the kinds that offer the command (`false`: it is on every table, and a kind without
/// the command ignores it).
struct Verb {
    command: CommandId,
    key: &'static str,
    action: &'static str,
    kind_scoped: bool,
}

const fn verb(command: CommandId, key: &'static str, action: &'static str) -> Verb {
    Verb {
        command,
        key,
        action,
        kind_scoped: true,
    }
}

const VERBS: &[Verb] = &[
    verb(
        CommandId::WORKLOAD_SCALE,
        "s",
        "resource_table::ScaleSelected",
    ),
    Verb {
        kind_scoped: false,
        ..verb(CommandId::POD_SHELL, "s", "resource_table::ShellSelected")
    },
    Verb {
        kind_scoped: false,
        ..verb(CommandId::NODE_SHELL, "s", "resource_table::ShellSelected")
    },
    verb(
        CommandId::CRONJOB_SUSPEND,
        "s",
        "resource_table::ToggleSuspendSelected",
    ),
    verb(
        CommandId::JOB_SUSPEND,
        "s",
        "resource_table::ToggleSuspendSelected",
    ),
    verb(
        CommandId::WORKLOAD_RESTART,
        "r",
        "resource_table::RestartSelected",
    ),
    verb(CommandId::NODE_DRAIN, "r", "resource_table::DrainSelected"),
    verb(
        CommandId::CRONJOB_TRIGGER,
        "t",
        "resource_table::TriggerSelected",
    ),
    verb(
        CommandId::POD_SANITIZE,
        "z",
        "resource_table::SanitizeSelected",
    ),
    verb(
        CommandId::SECRET_REVEAL,
        "x",
        "resource_table::RevealSelected",
    ),
    verb(
        CommandId::WORKLOAD_SET_IMAGE,
        "i",
        "resource_table::SetImageSelected",
    ),
    verb(
        CommandId::WORKLOAD_ROLLBACK,
        "ctrl-l",
        "resource_table::RollbackSelected",
    ),
    verb(
        CommandId::NODE_CORDON,
        "c",
        "resource_table::CordonSelected",
    ),
    verb(
        CommandId::NODE_UNCORDON,
        "u",
        "resource_table::UncordonSelected",
    ),
    verb(
        CommandId::NAMESPACE_SELECT,
        "u",
        "resource_table::UseNamespace",
    ),
    verb(
        CommandId::SERVICE_SHOW_PODS,
        "u",
        "resource_table::ShowServicePods",
    ),
    verb(
        CommandId::RESOURCE_SHOW_USED_BY,
        "u",
        "resource_table::ShowUsedBy",
    ),
];

/// What `keys` names in the table of `kind` with one row selected.
fn resolved(cx: &mut TestAppContext, kind: &str, keys: &str) -> Option<String> {
    let stack = parse_stack(&[
        "Workspace",
        "ClusterTab connected",
        "Workspace",
        "Pane",
        &format!("ResourceTable kind={kind} selection=one"),
    ])
    .expect("a context stack");
    cx.read(|cx| resolve(cx, keys, &stack).expect("resolves"))
        .action()
        .map(str::to_owned)
}

fn offers(case: &KindCase, command: CommandId) -> bool {
    case.actions.contains(&command)
}

/// Whether `case` offers a command that `action` runs (`s` suspends a Job and a CronJob alike).
fn offers_action(case: &KindCase, action: &str) -> bool {
    VERBS
        .iter()
        .any(|verb| verb.action == action && offers(case, verb.command))
}

#[gpui::test]
fn every_k9s_verb_is_bound_on_the_kinds_that_offer_it(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test())).expect("init");
    let mut bound = 0;
    for verb in VERBS {
        let kinds: Vec<&KindCase> = catalog().filter(|c| offers(c, verb.command)).collect();
        assert!(!kinds.is_empty(), "no kind offers {}", verb.command);
        for case in catalog() {
            let action = resolved(cx, case.kind, verb.key);
            if offers(case, verb.command) {
                assert_eq!(
                    action.as_deref(),
                    Some(verb.action),
                    "`{}` on a {} runs {}",
                    verb.key,
                    case.kind,
                    verb.command
                );
                bound += 1;
            } else if verb.kind_scoped && !offers_action(case, verb.action) {
                assert_ne!(
                    action.as_deref(),
                    Some(verb.action),
                    "`{}` on a {}, which offers no command {} runs",
                    verb.key,
                    case.kind,
                    verb.action
                );
            }
        }
    }
    assert!(bound >= 20, "{bound} kind bindings checked");
}

/// A ServiceAccount (an Access Control kind, E17, outside this matrix) is one of the kinds
/// k9s' `u` "used by" is for (E12-S18), as are the matrix's ConfigMaps, Secrets, claims and
/// PriorityClasses.
#[gpui::test]
fn used_by_is_bound_on_service_accounts_too(cx: &mut TestAppContext) {
    cx.update(|cx| init(cx, StartupEnv::test())).expect("init");
    assert_eq!(
        resolved(cx, "ServiceAccount", "u").as_deref(),
        Some("resource_table::ShowUsedBy")
    );
}
