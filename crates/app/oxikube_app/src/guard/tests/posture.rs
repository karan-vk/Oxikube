//! The posture commands: read-only toggle, colour and presets. They write the settings, follow
//! into the live session, confirm before lifting read-only on a production-flagged cluster, and
//! are audited.

use futures::{FutureExt as _, StreamExt as _};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::{ClusterColour, ClusterPreset, ErrorKind, OxiError};
use oxikube_ports::ClusterPrefs;

use crate::command_bus::{DispatchError, Outcome};
use crate::guard::PrefsPatch;
use crate::session::SessionChange;
use crate::testing::{Harness, ctx, id};
use crate::testing_posture::RED;

fn toggle(name: &str, read_only: Option<bool>) -> Command {
    Command::ClusterToggleReadOnly {
        cluster: id(name),
        read_only,
    }
}

fn preset(name: &str, preset: ClusterPreset) -> Command {
    Command::ClusterApplyPreset {
        cluster: id(name),
        preset,
    }
}

fn colour(name: &str, colour: Option<ClusterColour>) -> Command {
    Command::ClusterSetColour {
        cluster: id(name),
        colour,
    }
}

fn read_only(h: &Harness, name: &str) -> bool {
    h.manager.get(&id(name)).unwrap().read_only()
}

/// A connected, writable cluster `a` that the user flagged as production.
fn prod_cluster() -> Harness {
    let h = Harness::new();
    h.prefs.seed(
        &id("a"),
        ClusterPrefs {
            colour: Some(RED),
            ..ClusterPrefs::default()
        },
    );
    h.connect_configured("a");
    h
}

#[test]
fn turning_read_only_on_goes_live_is_saved_and_audited() {
    let h = Harness::new();
    h.connect("a", false);
    let mut updates = h.manager.subscribe();

    let out = h
        .dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    assert!(matches!(out, Outcome::Completed(_)), "{out:?}");

    assert!(read_only(&h, "a"));
    assert!(h.prefs.stored(&id("a")).read_only, "the setting follows");
    assert_eq!(
        h.prefs.writes(),
        [(
            id("a"),
            Some("a".to_owned()),
            PrefsPatch {
                read_only: Some(true),
                colour: None
            }
        )],
        "one write, naming the context so a new block is recognisable"
    );
    // The session announces the change once, even though the settings echo arrives too.
    let mut seen = Vec::new();
    while let Some(Some(Ok(update))) = updates.next().now_or_never() {
        seen.push(update.change);
    }
    let changes = seen
        .iter()
        .filter(|c| matches!(c, SessionChange::ReadOnlyChanged(_)))
        .count();
    assert_eq!(changes, 1, "{seen:?}");

    let audit = h.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].cmd.as_ref(), "cluster::ToggleReadOnly");
    assert_eq!(audit[0].outcome, AuditOutcome::Succeeded);
    assert_eq!(audit[0].initiator, Initiator::Ui);
    assert_eq!(audit[0].cluster, id("a"));
}

#[test]
fn toggling_without_a_value_flips_the_current_one() {
    let h = Harness::new();
    h.connect("a", false);
    h.dispatch(toggle("a", None), ctx(Initiator::Command))
        .unwrap();
    assert!(read_only(&h, "a"));
    h.dispatch(toggle("a", None), ctx(Initiator::Command))
        .unwrap();
    assert!(!read_only(&h, "a"));
    assert!(!h.prefs.stored(&id("a")).read_only);
    assert_eq!(h.audit().len(), 2);
}

#[test]
fn turning_read_only_off_updates_the_setting_and_the_live_session() {
    let h = Harness::new();
    h.prefs.seed(
        &id("a"),
        ClusterPrefs {
            read_only: true,
            ..ClusterPrefs::default()
        },
    );
    h.connect_configured("a");
    assert!(read_only(&h, "a"));

    let out = h
        .dispatch(toggle("a", Some(false)), ctx(Initiator::Ui))
        .unwrap();
    assert!(
        matches!(out, Outcome::Completed(_)),
        "no confirm: not flagged production"
    );

    assert!(!read_only(&h, "a"));
    assert!(!h.prefs.stored(&id("a")).read_only);
    // A write now reaches the cluster: the flag really is off.
    h.allow_deletes("a", 1);
    let del = crate::testing::pod_delete("a", "web-0");
    assert!(matches!(
        h.confirm_and_run(del, ctx(Initiator::Ui)),
        Ok(Outcome::Completed(_))
    ));
}

#[test]
fn the_live_session_follows_even_when_the_settings_echo_never_arrives() {
    let h = Harness::new();
    h.prefs
        .echo
        .store(false, std::sync::atomic::Ordering::SeqCst);
    h.connect("a", true);
    h.dispatch(toggle("a", Some(false)), ctx(Initiator::Ui))
        .unwrap();
    assert!(!read_only(&h, "a"), "the command applies the value itself");
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    assert!(read_only(&h, "a"));
}

#[test]
fn lifting_read_only_on_a_production_cluster_asks_for_a_simple_confirm() {
    let h = prod_cluster();
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    assert!(read_only(&h, "a"));
    let writes_before = h.prefs.writes().len();

    let request = h.ask(toggle("a", Some(false)), ctx(Initiator::Ui));
    assert_eq!(request.command, CommandId::CLUSTER_TOGGLE_READ_ONLY);
    assert_eq!(request.tier, oxikube_domain::safety::ConfirmTier::Simple);
    assert_eq!(request.cluster, id("a"));
    assert!(
        request.summary.contains("production"),
        "{}",
        request.summary
    );
    assert!(request.summary.contains('a'));
    assert_eq!(request.expected_name, None);
    assert!(read_only(&h, "a"), "nothing happened yet");
    assert_eq!(h.prefs.writes().len(), writes_before);

    let out = h.confirm_and_run(toggle("a", Some(false)), ctx(Initiator::Ui));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert!(!read_only(&h, "a"));
    assert!(!h.prefs.stored(&id("a")).read_only);
    let outcomes: Vec<_> = h.audit().iter().map(|r| r.outcome).collect();
    assert_eq!(outcomes, [AuditOutcome::Succeeded, AuditOutcome::Succeeded]);
}

#[test]
fn declining_the_production_confirm_changes_nothing_and_is_audited() {
    let h = prod_cluster();
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    let request = h.ask(toggle("a", Some(false)), ctx(Initiator::Command));
    h.bus
        .decline(request.token)
        .now_or_never()
        .unwrap()
        .unwrap();
    assert!(read_only(&h, "a"));
    assert!(h.prefs.stored(&id("a")).read_only);
    let last = h.audit().pop().unwrap();
    assert_eq!(last.outcome, AuditOutcome::Cancelled);
    assert_eq!(last.cmd.as_ref(), "cluster::ToggleReadOnly");
}

#[test]
fn a_confirmation_for_another_request_does_not_lift_read_only() {
    let h = prod_cluster();
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    let request = h.ask(toggle("a", None), ctx(Initiator::Ui));
    let err = h
        .dispatch(
            toggle("a", Some(false)),
            ctx(Initiator::Ui).with_confirmation(crate::Confirmation::simple(request.token)),
        )
        .unwrap_err();
    assert!(matches!(err, DispatchError::Confirmation(_)), "{err:?}");
    assert!(read_only(&h, "a"));
    assert_eq!(h.audit().last().unwrap().outcome, AuditOutcome::Denied);
}

#[test]
fn no_confirm_to_turn_it_on_or_for_a_cluster_that_is_not_flagged() {
    let h = prod_cluster();
    // Turning on never asks, even on a production cluster.
    assert!(matches!(
        h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui)),
        Ok(Outcome::Completed(_))
    ));
    // Unflagging production first is a person's explicit choice, and then lifting is plain.
    h.dispatch(colour("a", None), ctx(Initiator::Ui)).unwrap();
    assert!(matches!(
        h.dispatch(toggle("a", Some(false)), ctx(Initiator::Ui)),
        Ok(Outcome::Completed(_))
    ));
}

#[test]
fn agents_and_plugins_cannot_lift_read_only_even_with_a_confirmation() {
    let h = prod_cluster();
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    for initiator in [Initiator::Agent, Initiator::Plugin] {
        let err = h
            .dispatch(toggle("a", Some(false)), ctx(initiator))
            .unwrap_err();
        assert!(matches!(err, DispatchError::NotPermitted { .. }), "{err:?}");
        assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
    }
    assert!(read_only(&h, "a"));
    assert!(h.bus.tool(CommandId::CLUSTER_TOGGLE_READ_ONLY).is_none());
}

/// Every command that takes the production colour off a cluster flagged with it.
fn unflagging_commands() -> Vec<Command> {
    vec![
        colour("a", None),
        colour("a", Some(ClusterColour::rgb(1, 2, 3))),
        preset("a", ClusterPreset::Staging),
        preset("a", ClusterPreset::Dev),
        preset("a", ClusterPreset::None),
    ]
}

#[test]
fn agents_and_plugins_cannot_clear_the_production_flag() {
    for initiator in [Initiator::Agent, Initiator::Plugin] {
        for command in unflagging_commands() {
            let h = prod_cluster();
            let err = h.dispatch(command.clone(), ctx(initiator)).unwrap_err();
            assert!(
                matches!(err, DispatchError::NotPermitted { .. }),
                "{initiator} {command:?}: {err:?}"
            );
            assert_eq!(OxiError::from(err).kind(), ErrorKind::Forbidden);
            assert_eq!(h.manager.get(&id("a")).unwrap().colour(), Some(RED));
            assert!(h.prefs.writes().is_empty(), "nothing was written");
            assert_eq!(h.outcomes(), [(AuditOutcome::Denied, initiator)]);
        }
    }
}

#[test]
fn people_may_clear_the_production_flag_and_agents_may_keep_or_set_it() {
    for command in unflagging_commands() {
        for initiator in [Initiator::Ui, Initiator::Command] {
            let h = prod_cluster();
            let out = h.dispatch(command.clone(), ctx(initiator));
            assert!(
                matches!(out, Ok(Outcome::Completed(_))),
                "{command:?}: {out:?}"
            );
            assert_ne!(h.manager.get(&id("a")).unwrap().colour(), Some(RED));
        }
    }
    for initiator in [Initiator::Agent, Initiator::Plugin] {
        // Keeping the flag, or setting it, never lowers anything.
        let h = prod_cluster();
        for command in [colour("a", Some(RED)), preset("a", ClusterPreset::Prod)] {
            let out = h.dispatch(command.clone(), ctx(initiator));
            assert!(
                matches!(out, Ok(Outcome::Completed(_))),
                "{command:?}: {out:?}"
            );
        }
        // A cluster that is not flagged production can be recoloured freely.
        let h = Harness::new();
        h.connect("a", false);
        for command in [
            colour("a", Some(ClusterColour::rgb(1, 2, 3))),
            colour("a", None),
        ] {
            let out = h.dispatch(command.clone(), ctx(initiator));
            assert!(
                matches!(out, Ok(Outcome::Completed(_))),
                "{command:?}: {out:?}"
            );
        }
    }
}

#[test]
fn clearing_the_production_flag_needs_a_writable_audit_log() {
    let h = prod_cluster();
    // A record that could not be written leaves a backlog (raising protection still ran).
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(h.bus.guard().audit().backlog_len(), 1);

    // While the log stays unwritable, clearing the flag is refused before anything changes.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let writes = h.prefs.writes().len();
    let err = h
        .dispatch(colour("a", None), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::AuditUnavailable(_)), "{err:?}");
    assert_eq!(h.manager.get(&id("a")).unwrap().colour(), Some(RED));
    assert_eq!(h.prefs.writes().len(), writes, "nothing was written");
}

#[test]
fn a_failed_save_never_lowers_protection() {
    let h = Harness::new();
    h.connect("a", true);
    h.prefs
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let err = h
        .dispatch(toggle("a", Some(false)), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::Handler(_)), "{err:?}");
    assert!(read_only(&h, "a"), "still read-only");
    assert_eq!(h.audit().last().unwrap().outcome, AuditOutcome::Failed);
}

#[test]
fn a_failed_save_never_loses_protection_that_was_asked_for() {
    let h = Harness::new();
    h.connect("a", false);
    h.prefs
        .fail
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let err = h
        .dispatch(toggle("a", Some(true)), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(read_only(&h, "a"), "on for this session");
    let message = OxiError::from(err).message().to_owned();
    assert!(message.contains("could not be saved"), "{message}");
    assert!(message.contains("disk full"), "{message}");
}

#[test]
fn lowering_protection_needs_a_writable_audit_log_raising_it_does_not() {
    let h = Harness::new();
    h.connect("a", false);

    // Raising protection never waits on the log: it goes through and the record is kept.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let out = h.dispatch(toggle("a", Some(true)), ctx(Initiator::Ui));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert!(read_only(&h, "a"));
    assert_eq!(h.bus.guard().audit().backlog_len(), 1);

    // While the log stays unwritable, lifting read-only is refused before anything changes.
    h.state
        .script()
        .append_audit
        .push_err(OxiError::internal("disk full"));
    let writes = h.prefs.writes().len();
    let err = h
        .dispatch(toggle("a", Some(false)), ctx(Initiator::Ui))
        .unwrap_err();
    assert!(matches!(err, DispatchError::AuditUnavailable(_)), "{err:?}");
    assert!(read_only(&h, "a"));
    assert_eq!(h.prefs.writes().len(), writes, "nothing was written");

    // Once the store recovers, the backlog lands first and the change goes through.
    let out = h.dispatch(toggle("a", Some(false)), ctx(Initiator::Ui));
    assert!(matches!(out, Ok(Outcome::Completed(_))), "{out:?}");
    assert!(!read_only(&h, "a"));
    assert_eq!(h.bus.guard().audit().backlog_len(), 0);
    assert_eq!(h.audit().len(), 2);
}

#[test]
fn presets_write_exactly_their_fields() {
    let h = Harness::new();
    h.connect("a", false);

    h.dispatch(preset("a", ClusterPreset::Prod), ctx(Initiator::Ui))
        .unwrap();
    let session = h.manager.get(&id("a")).unwrap();
    assert_eq!(session.colour(), Some(ClusterPreset::PROD_COLOUR));
    assert!(session.read_only());
    assert_eq!(
        h.prefs.writes().last().unwrap().2,
        PrefsPatch {
            read_only: Some(true),
            colour: Some(Some(ClusterPreset::PROD_COLOUR))
        }
    );

    // Staging and dev only recolour: they never lower protection.
    for (p, want) in [
        (ClusterPreset::Staging, ClusterPreset::STAGING_COLOUR),
        (ClusterPreset::Dev, ClusterPreset::DEV_COLOUR),
    ] {
        h.dispatch(preset("a", p), ctx(Initiator::Ui)).unwrap();
        let session = h.manager.get(&id("a")).unwrap();
        assert_eq!(session.colour(), Some(want));
        assert!(session.read_only(), "{p:?} leaves read-only as it was");
        assert_eq!(
            h.prefs.writes().last().unwrap().2,
            PrefsPatch {
                read_only: None,
                colour: Some(Some(want))
            }
        );
    }

    h.dispatch(preset("a", ClusterPreset::None), ctx(Initiator::Ui))
        .unwrap();
    assert_eq!(h.manager.get(&id("a")).unwrap().colour(), None);
    assert_eq!(h.prefs.stored(&id("a")).colour, None);
    assert_eq!(h.audit().len(), 4);
}

#[test]
fn colour_runs_on_a_read_only_cluster_for_every_initiator_and_is_audited() {
    for initiator in Initiator::ALL {
        let h = Harness::new();
        h.connect("a", true);
        let out = h.dispatch(colour("a", Some(RED)), ctx(initiator));
        assert!(
            matches!(out, Ok(Outcome::Completed(_))),
            "{initiator}: {out:?}"
        );
        assert_eq!(h.manager.get(&id("a")).unwrap().colour(), Some(RED));
        assert_eq!(h.outcomes(), [(AuditOutcome::Succeeded, initiator)]);
    }
}

#[test]
fn a_cluster_without_a_session_is_configured_through_its_settings() {
    let h = Harness::new();
    h.dispatch(preset("a", ClusterPreset::Prod), ctx(Initiator::Ui))
        .unwrap();
    assert!(h.prefs.stored(&id("a")).read_only);
    assert_eq!(
        h.prefs.writes()[0].1,
        None,
        "no context name without a session"
    );
    // A session opened afterwards starts from it.
    h.connect_configured("a");
    let session = h.manager.get(&id("a")).unwrap();
    assert!(session.read_only());
    assert_eq!(session.colour(), Some(ClusterPreset::PROD_COLOUR));
}

#[test]
fn colour_and_preset_commands_have_tool_stubs() {
    let h = Harness::new();
    for (id, name) in [
        (CommandId::CLUSTER_SET_COLOUR, "app.cluster_set_colour"),
        (CommandId::CLUSTER_APPLY_PRESET, "app.cluster_apply_preset"),
    ] {
        let tool = h.bus.tool(id).unwrap_or_else(|| panic!("{id} has no tool"));
        assert_eq!(tool.name.as_str(), name);
        assert!(!tool.is_mutating(), "it never changes cluster state");
    }
}

#[test]
fn a_dry_run_previews_a_posture_change_and_changes_nothing() {
    let h = prod_cluster();
    let before = h.prefs.stored(&id("a"));
    let mut updates = h.manager.subscribe();

    let commands = [
        preset("a", ClusterPreset::Prod),
        colour("a", None),
        toggle("a", Some(true)),
    ];
    for command in commands {
        let out = h
            .dispatch(command, ctx(Initiator::Ui).with_dry_run(true))
            .unwrap();
        let Outcome::Completed(output) = out else {
            panic!("expected a completed preview, got {out:?}");
        };
        let data = output.data.expect("a preview describes the outcome");
        assert_eq!(data["dry_run"], true, "{data}");
        assert!(output.message.unwrap().starts_with("Dry run:"));
    }

    assert!(h.prefs.writes().is_empty(), "nothing is saved");
    assert_eq!(h.prefs.stored(&id("a")), before);
    let session = h.manager.get(&id("a")).unwrap();
    assert!(!session.read_only(), "the live session is untouched");
    assert_eq!(session.colour(), Some(RED), "the production flag survives");
    assert!(
        updates.next().now_or_never().is_none(),
        "no change is announced"
    );

    let audit = h.audit();
    assert_eq!(audit.len(), 3);
    assert!(audit.iter().all(|r| r.dry_run), "{audit:?}");
}

#[test]
fn a_dry_run_reports_what_the_preset_would_leave_behind() {
    let h = Harness::new();
    h.connect("a", false);
    let out = h
        .dispatch(
            preset("a", ClusterPreset::Prod),
            ctx(Initiator::Ui).with_dry_run(true),
        )
        .unwrap();
    let Outcome::Completed(output) = out else {
        panic!("{out:?}");
    };
    let data = output.data.unwrap();
    assert_eq!(data["read_only"], true);
    assert_eq!(
        data["colour"],
        serde_json::json!(ClusterPreset::PROD_COLOUR)
    );
    assert!(!read_only(&h, "a"));
}
