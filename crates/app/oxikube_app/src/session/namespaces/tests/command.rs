//! `namespace::Select` and `namespace::ToggleFavourite` as commands with tool stubs.

use oxikube_domain::command::{Command, CommandId, lookup};
use oxikube_domain::session::NamespaceSelection;
use oxikube_domain::{ErrorKind, command::COMMANDS};

use super::*;

#[test]
fn both_commands_are_registered_read_only_with_tool_names() {
    for (command, tool) in [
        (CommandId::NAMESPACE_SELECT, "app.namespace_select"),
        (
            CommandId::NAMESPACE_TOGGLE_FAVOURITE,
            "app.namespace_toggle_favourite",
        ),
    ] {
        let meta = lookup(command).expect("registered");
        assert!(!meta.mutating, "{command} only changes the view");
        assert!(!meta.privileged);
        assert_eq!(command.tool_name(), tool);
        assert!(COMMANDS.iter().any(|m| m.id == command));
    }
}

#[test]
fn select_runs_through_the_service() {
    let mut h = Harness::new();
    h.connect("a", &["dev", "prod"]);
    h.namespace_changes();

    let outcome = h
        .run(h.service.execute(&Command::NamespaceSelect {
            cluster: id("a"),
            namespaces: vec!["prod".into(), "dev".into()],
        }))
        .unwrap();

    assert!(outcome.changed);
    assert_eq!(
        h.selection("a"),
        NamespaceSelection::from_names(["dev", "prod"])
    );
    assert_eq!(h.namespace_changes().len(), 1);
}

#[test]
fn an_empty_namespace_list_selects_all() {
    let h = Harness::new();
    h.connect("a", &["dev"]);
    h.run(
        h.service
            .select(&id("a"), NamespaceSelection::single("dev")),
    )
    .unwrap();

    h.run(h.service.execute(&Command::NamespaceSelect {
        cluster: id("a"),
        namespaces: vec![],
    }))
    .unwrap();

    assert_eq!(h.selection("a"), NamespaceSelection::All);
}

#[test]
fn toggle_favourite_runs_through_the_service() {
    let h = Harness::new();
    let command = Command::NamespaceToggleFavourite {
        cluster: id("a"),
        namespace: "prod".into(),
    };

    let on = h.run(h.service.execute(&command)).unwrap();
    assert!(on.prefs.favourites.contains("prod"));
    let off = h.run(h.service.execute(&command)).unwrap();
    assert!(!off.prefs.favourites.contains("prod"));
}

#[test]
fn a_blank_favourite_is_rejected() {
    let h = Harness::new();
    let err = h
        .run(h.service.execute(&Command::NamespaceToggleFavourite {
            cluster: id("a"),
            namespace: "  ".into(),
        }))
        .expect_err("blank");
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn other_commands_are_not_handled() {
    let h = Harness::new();
    let err = h
        .run(h.service.execute(&Command::PaletteToggle))
        .expect_err("not a namespace command");
    assert_eq!(err.kind(), ErrorKind::Unsupported);
}
