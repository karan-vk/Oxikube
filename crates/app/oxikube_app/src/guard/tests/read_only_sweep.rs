//! The read-only sweep (E19-S11): every declared mutating command, through every door, against a
//! read-only cluster, over the real command handlers and the `oxikube_testkit` fakes.
//!
//! `enforcement.rs` proves the bus denies a recording handler. This sweep goes further:
//!
//! * the handlers that can be built from fakes are the real ones (`resource::Delete`, the
//!   workload, node, RBAC, namespace, storage, apply and Helm operation handlers). On a
//!   read-only cluster the guard denies a mutating command before any handler body runs, so
//!   those handlers are only registered here, not executed: this sweep proves the *guard*
//!   denies every mutating command, not that a handler body avoids the writer. Handler bodies
//!   run only in the reverse check (non-mutating commands) and the positive control
//!   (`resource::Delete`). Running every real mutating handler on a writable cluster to catch a
//!   direct `writer()` call is tracked as a follow-up and covered statically by the E19-S03
//!   mutation lint;
//! * after every attempt it checks the audit log (exactly one `Denied` record naming the
//!   command, the initiator and, when recorded, the read-only reason) and every fake port
//!   (zero writes on the resource port, zero calls on the exec and port-forward fakes, no
//!   mutating Helm call);
//! * the reverse check runs every *non*-mutating command on a writable cluster and fails when
//!   one of them wrote, which is how a mutating command that is not marked `mutating` shows up;
//! * a positive control proves the fakes record a permitted write, and meta-tests prove the
//!   sweep fails on a bypassing command, a missing sample and a writable cluster;
//! * every failure of a run is collected and reported at once.
//!
//! # Adding a command
//!
//! 1. Declare it in `oxikube_domain::command` with the right `mutating` flag.
//! 2. Add an arm to `testing_posture::sample` (the per-command argument factory). Without it
//!    this sweep fails and names the command.
//! 3. Run `cargo test -p oxikube_app read_only_sweep`.
//!
//! A command that cannot be swept yet goes in [`SKIPPED`] with a reason; the list is empty today
//! and `the_skip_list_is_empty` keeps it that way unless someone edits both on purpose.

use std::fmt::Write as _;
use std::sync::Arc;

use oxikube_domain::audit::{AuditDenial, AuditOutcome, Initiator};
use oxikube_domain::command::{COMMANDS, Command, CommandId, CommandMeta};
use oxikube_domain::ids::Gvk;
use oxikube_ports::DeleteOptions;
use oxikube_testkit::{FakeHelmPort, ResourceCall};
use parking_lot::Mutex;

use super::enforcement::tool_call;
use crate::command_bus::{
    CommandOutput, CommandRegistry, DispatchContext, DispatchError, HandlerContext, RegisterError,
};
use crate::session::ClusterSessionManager;
use crate::testing::{Calls, Harness, MUTATING, ctx, id, register_recording_rest};
use crate::testing_posture::try_sample;

/// Commands the sweep does not run, each with the reason. Must stay empty: a command that cannot
/// be swept is a gap in read-only mode, not a case to wave through.
const SKIPPED: &[(CommandId, &str)] = &[];

/// A way a request reaches the guard.
#[derive(Debug, Clone, Copy)]
enum Door {
    /// `CommandBus::dispatch` as this initiator (the UI, a keybinding, the palette, a plugin).
    Direct(Initiator),
    /// What an MCP `tools/call` does: the tool name picks the command, the arguments are its
    /// payload, the initiator is the agent. Dispatches through the registered tool stub.
    Tool,
}

/// The doors under test.
///
/// EXTENSION POINT (E26/E27): when the real MCP `ToolRegistry` exists, add a `Door` variant that
/// calls it and list it here; the sweep needs no other change.
fn doors() -> Vec<Door> {
    let mut doors: Vec<Door> = Initiator::ALL.into_iter().map(Door::Direct).collect();
    doors.push(Door::Tool);
    doors
}

/// Builds the command to sweep for an id, or `None` when no sample exists.
type Factory<'a> = &'a dyn Fn(CommandId) -> Option<Command>;

/// The production factory: `testing_posture::try_sample` aimed at cluster `a`; a missing sample
/// becomes a failure line, so one run lists them all.
fn samples(command: CommandId) -> Option<Command> {
    try_sample(command, "a")
}

/// A harness with the real handlers that can be built from fakes, and a recording handler for
/// every other declared command.
struct World {
    h: Harness,
    helm: Arc<FakeHelmPort>,
    /// The commands whose handler is a real one.
    real: Vec<CommandId>,
}

impl World {
    fn new() -> Self {
        Self::with_extra(|_, _| Ok(()))
    }

    /// `extra` registers handlers before the rest are filled in (a meta-test installs a
    /// misbehaving one this way).
    fn with_extra(
        extra: impl FnOnce(&mut CommandRegistry, &ClusterSessionManager) -> Result<(), RegisterError>,
    ) -> Self {
        let helm = Arc::new(FakeHelmPort::new());
        let real = Arc::new(Mutex::new(Vec::new()));
        let (helm_in, real_in) = (helm.clone(), real.clone());
        let h = Harness::build(
            &MUTATING,
            move |reg: &mut CommandRegistry, calls: &Calls, sessions| {
                crate::actions::register_commands(reg)?;
                crate::workloads::register_commands(reg, sessions.clone())?;
                crate::nodes::register_commands(reg)?;
                crate::selection::register_commands(reg, sessions.clone())?;
                crate::ingress_class::register_commands(reg, sessions.clone())?;
                crate::storage::register_commands(reg, sessions.clone())?;
                crate::apply::register_commands(reg, sessions.clone())?;
                crate::namespaces::register_commands(reg)?;
                crate::role_ops::register_commands(reg)?;
                crate::binding_ops::register_commands(reg)?;
                crate::helm_ops::register_commands(reg, helm_in)?;
                extra(reg, sessions)?;
                real_in
                    .lock()
                    .extend(COMMANDS.iter().map(|m| m.id).filter(|id| reg.contains(*id)));
                register_recording_rest(reg, calls)
            },
        );
        let real = real.lock().clone();
        Self { h, helm, real }
    }

    /// Every write-class call the fake ports of `cluster` saw, as readable lines.
    fn port_writes(&self, cluster: &str) -> Vec<String> {
        let ports = self.h.connector.ports_for(&id(cluster));
        let mut seen: Vec<String> = ports
            .resources
            .mutating_calls()
            .iter()
            .map(|c| format!("resource port: {c:?}"))
            .collect();
        seen.extend(
            ports
                .exec
                .recorded_calls()
                .iter()
                .map(|c| format!("exec port: {c:?}")),
        );
        seen.extend(
            ports
                .port_forward
                .recorded_calls()
                .iter()
                .map(|c| format!("port-forward port: {c:?}")),
        );
        seen.extend(
            self.helm
                .recorded_calls()
                .iter()
                .filter(|c| c.is_mutating())
                .map(|c| format!("helm port: {c:?}")),
        );
        seen
    }
}

fn mutating() -> impl Iterator<Item = &'static CommandMeta> {
    COMMANDS
        .iter()
        .filter(|m| m.mutating && !SKIPPED.iter().any(|(id, _)| *id == m.id))
}

/// Dispatches `command` through `door`.
fn run(world: &World, meta: &CommandMeta, command: Command, door: Door) -> Result<(), String> {
    let (command, cx) = match door {
        Door::Direct(initiator) => (command, ctx(initiator)),
        Door::Tool => {
            let Some(tool) = world.h.bus.tool(meta.id) else {
                return Err("has no MCP tool stub".to_owned());
            };
            let mut arguments = serde_json::to_value(&command).map_err(|e| e.to_string())?;
            if let Some(object) = arguments.as_object_mut() {
                object.remove("type");
            }
            let command = tool_call(tool.name.as_str(), arguments);
            (command, DispatchContext::new(Initiator::Agent, "claude"))
        }
    };
    match world.h.dispatch(command, cx) {
        Err(DispatchError::ReadOnly { cluster, .. }) if cluster == id("a") => Ok(()),
        Err(other) => Err(format!("expected ReadOnly on cluster a, got {other:?}")),
        Ok(outcome) => Err(format!("expected ReadOnly, the command ran: {outcome:?}")),
    }
}

/// Runs the read-only sweep on cluster `a` of `world`, which the caller has connected read-only.
/// Returns one line per violation; empty means the sweep passed.
fn sweep(world: &World, factory: Factory<'_>, doors: &[Door]) -> Vec<String> {
    let mut failures = Vec::new();
    for meta in mutating() {
        let Some(command) = factory(meta.id) else {
            failures.push(format!(
                "{}: no sample command; add an arm for it to `testing_posture::sample` so the \
                 read-only sweep can run it",
                meta.id
            ));
            continue;
        };
        for door in doors {
            let label = format!("{} via {door:?}", meta.id);
            let audit_before = world.h.audit().len();
            let calls_before = world.h.calls().len();
            if let Err(why) = run(world, meta, command.clone(), *door) {
                failures.push(format!("{label}: {why}"));
            }
            let audit = world.h.audit();
            match &audit[audit_before.min(audit.len())..] {
                [record] => {
                    if record.outcome != AuditOutcome::Denied
                        || record.cmd.as_ref() != meta.id.as_str()
                        || record.denial.is_some_and(|d| d != AuditDenial::ReadOnly)
                        || !matches!(door, Door::Direct(i) if *i == record.initiator)
                            && !matches!(door, Door::Tool if record.initiator == Initiator::Agent)
                    {
                        failures.push(format!("{label}: wrong audit record {record:?}"));
                    }
                }
                records => failures.push(format!(
                    "{label}: expected one Denied audit record, found {}",
                    records.len()
                )),
            }
            if world.h.calls().len() != calls_before {
                failures.push(format!("{label}: a handler ran"));
            }
        }
    }
    for write in world.port_writes("a") {
        failures.push(format!(
            "a fake port was called on a read-only cluster: {write}"
        ));
    }
    failures
}

/// The reverse check: runs every command not marked `mutating` on a writable cluster `a` and
/// reports the ones that wrote through any write-class port.
fn reverse_check(world: &World, factory: Factory<'_>) -> Vec<String> {
    let mut failures = Vec::new();
    for meta in COMMANDS.iter().filter(|m| !m.mutating) {
        let Some(command) = factory(meta.id) else {
            failures.push(format!("{}: no sample command", meta.id));
            continue;
        };
        let before = world.port_writes("a").len();
        let _ = world.h.dispatch(command, ctx(Initiator::Ui));
        let after = world.port_writes("a");
        if after.len() > before {
            failures.push(format!(
                "{} is not marked `mutating` but wrote: {}; mark it mutating so the guard gates it",
                meta.id,
                after[before..].join("; ")
            ));
        }
    }
    failures
}

/// Fails with every collected violation at once.
#[track_caller]
fn assert_clean(what: &str, failures: &[String]) {
    if failures.is_empty() {
        return;
    }
    let mut report = format!("{what}: {} violation(s)\n", failures.len());
    for failure in failures {
        let _ = writeln!(report, "  - {failure}");
    }
    panic!("{report}");
}

fn read_only_world() -> World {
    let world = World::new();
    world.h.connect("a", true);
    world
}

#[test]
fn read_only_sweep() {
    let world = read_only_world();
    let attempts = mutating().count() * doors().len();
    assert!(attempts >= 100, "the sweep ran only {attempts} attempts");
    assert_clean("read-only sweep", &sweep(&world, &samples, &doors()));
    assert_eq!(
        world.h.audit().len(),
        attempts,
        "one audit record per attempt"
    );
    assert_eq!(world.h.bus.guard().pending_confirmations(), 0);
}

#[test]
fn the_sweep_registers_the_real_handlers_where_fakes_can_build_them() {
    let world = World::new();
    for expected in [
        CommandId::RESOURCE_DELETE,
        CommandId::RESOURCE_APPLY,
        CommandId::NODE_CORDON,
        CommandId::NODE_UNCORDON,
        CommandId::POD_DELETE,
        CommandId::WORKLOAD_SCALE,
        CommandId::HELM_ROLLBACK,
        CommandId::HELM_UNINSTALL,
        CommandId::ROLE_CREATE,
        CommandId::BINDING_DELETE,
        CommandId::NAMESPACE_CREATE,
    ] {
        assert!(
            world.real.contains(&expected),
            "{expected} has a recording handler in the sweep world, not its real one"
        );
    }
    assert!(world.real.len() >= 20, "{} real handlers", world.real.len());
}

#[test]
fn no_command_outside_the_mutating_set_writes() {
    let world = World::new();
    world.h.connect("a", false);
    assert_clean("reverse check", &reverse_check(&world, &samples));
}

#[test]
fn a_non_mutating_command_that_writes_is_detected() {
    // Meta-test: a `#[cfg(test)]` registration of a command declared non-mutating whose handler
    // takes the session's writer directly, without a `Mutation`. The sweep's reverse check must
    // name it.
    let rogue = COMMANDS
        .iter()
        .find(|m| m.id.as_str() == "events::ToggleFaults")
        .expect("a declared read command");
    assert!(!rogue.mutating);
    let world = World::with_extra(|reg, sessions| {
        let sessions = sessions.clone();
        reg.register(*rogue, move |_: Command, _: HandlerContext| {
            let writer = sessions.get(&id("a")).and_then(|s| s.writer());
            async move {
                if let Some(writer) = writer {
                    let _ = writer
                        .delete(
                            &Gvk::new("", "v1", "Pod"),
                            Some("default"),
                            "x",
                            &DeleteOptions::default(),
                        )
                        .await;
                }
                Ok(CommandOutput::none())
            }
        })
    });
    world.h.connect("a", false);
    let failures = reverse_check(&world, &samples);
    assert_eq!(failures.len(), 1, "{failures:#?}");
    assert!(failures[0].contains("events::ToggleFaults"), "{failures:?}");
    assert!(
        failures[0].contains("not marked `mutating`"),
        "{failures:?}"
    );
}

#[test]
fn a_command_without_a_sample_fails_the_sweep_with_an_actionable_message() {
    let world = read_only_world();
    let factory = |command: CommandId| {
        (command != CommandId::NODE_CORDON && command != CommandId::POD_DELETE)
            .then(|| samples(command))
            .flatten()
    };
    let failures = sweep(&world, &factory, &doors());
    // Both commands are listed in the one run.
    assert_eq!(failures.len(), 2, "{failures:#?}");
    for command in ["node::Cordon", "pod::Delete"] {
        let line = failures
            .iter()
            .find(|f| f.starts_with(command))
            .unwrap_or_else(|| panic!("{command} not reported: {failures:#?}"));
        assert!(line.contains("testing_posture::sample"), "{line}");
    }
}

#[test]
fn the_sweep_is_not_vacuous_on_a_writable_cluster() {
    // The control: with the flag off the same sweep reports every attempt as a violation (the
    // command is not denied for being read-only), so a pass above is the flag, not an empty
    // loop or a missing handler.
    let world = World::new();
    world.h.connect("a", false);
    let failures = sweep(&world, &samples, &doors());
    let attempts = mutating().count() * doors().len();
    assert!(
        failures.len() >= attempts,
        "{} violations for {attempts} attempts",
        failures.len()
    );
}

#[test]
fn a_permitted_command_writes_exactly_the_expected_requests() {
    // Positive control: read-only off, the user confirms, and the fake resource port records
    // exactly the requests the command means (the server dry run, then the delete), so "zero
    // writes" in the sweep is a real measurement of a port that does record.
    let h = Harness::with_delete_handler();
    h.connect("a", false);
    h.allow_deletes("a", 2);
    let delete = Command::ResourceDelete {
        target: crate::testing::pod("a", "web-0"),
        propagation: oxikube_domain::command::Propagation::Background,
    };
    h.confirm_and_run(delete, ctx(Initiator::Ui))
        .expect("the confirmed delete runs");
    let writes = h.writes("a");
    let shape: Vec<(bool, String)> = writes
        .iter()
        .map(|call| match call {
            ResourceCall::Delete { name, options, .. } => (options.dry_run, name.clone()),
            other => panic!("expected only Delete calls, got {other:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [(true, "web-0".to_owned()), (false, "web-0".to_owned())]
    );
    assert_eq!(
        h.outcomes().last(),
        Some(&(AuditOutcome::Succeeded, Initiator::Ui))
    );
}

#[test]
fn the_skip_list_is_empty() {
    assert!(
        SKIPPED.is_empty(),
        "the read-only sweep skips {:?}; fix the gap instead of listing it",
        SKIPPED
    );
}
