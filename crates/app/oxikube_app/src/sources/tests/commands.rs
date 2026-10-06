//! The `kubeconfig::*` command handlers and their tool stubs.

use oxikube_domain::ErrorKind;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{
    Command, CommandId, KubeconfigSourceRef, NewKubeconfigSource, PastedText,
};

use super::*;
use crate::command_bus::CommandRegistry;
use crate::sources::register_commands;

const SECRET: &str = "s3cr3t-token-do-not-leak";

fn add_file(path: &str) -> Command {
    Command::KubeconfigAddSource {
        source: NewKubeconfigSource::File { path: path.into() },
    }
}

#[test]
fn the_three_commands_register_with_tool_stubs() {
    let f = Fixture::with_defaults();
    let mut registry = CommandRegistry::new();
    registry
        .install("oxikube_catalog_ui", |registry| {
            register_commands(registry, &f.service)
        })
        .unwrap();
    assert_eq!(registry.len(), 3);
    for (id, tool) in [
        (
            CommandId::KUBECONFIG_ADD_SOURCE,
            "app.kubeconfig_add_source",
        ),
        (
            CommandId::KUBECONFIG_REMOVE_SOURCE,
            "app.kubeconfig_remove_source",
        ),
        (CommandId::KUBECONFIG_RELOAD, "app.kubeconfig_reload"),
    ] {
        assert!(registry.contains(id), "{id}");
        let stub = registry.entries[&id].tool.as_ref().expect("a tool stub");
        assert_eq!(stub.name.as_str(), tool);
    }
}

#[test]
fn add_and_reload_run_through_execute_and_return_rows_without_credentials() {
    let f = Fixture::with_defaults();
    let paste = Command::KubeconfigAddSource {
        source: NewKubeconfigSource::Pasted {
            name: "prod".into(),
            text: PastedText::new(format!("{}# {SECRET}\n", kubeconfig(1))),
        },
    };
    let output = f.run(f.service.execute(&paste, Initiator::Ui)).unwrap();
    assert!(output.message.unwrap().contains("1 context)"));
    let data = output.data.unwrap().to_string();
    assert!(data.contains("prod.yaml"), "{data}");
    assert!(!data.contains(SECRET), "output must not carry the text");

    let output = f
        .run(
            f.service
                .execute(&Command::KubeconfigReload, Initiator::Agent),
        )
        .unwrap();
    assert!(output.message.unwrap().starts_with("Reloaded kubeconfigs"));
}

#[test]
fn an_agent_may_reload_and_add_but_cannot_delete_a_stored_kubeconfig() {
    let path = stored("prod.yaml");
    let f = Fixture::new([UserSource::default_source(), UserSource::file(&path)]);
    f.fs.insert(path.clone(), kubeconfig(1).into_bytes());
    let remove = Command::KubeconfigRemoveSource {
        source: KubeconfigSourceRef::File {
            path: path.display().to_string(),
        },
    };

    for initiator in [Initiator::Agent, Initiator::Plugin] {
        let error = f.run(f.service.execute(&remove, initiator)).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Forbidden, "{initiator:?}");
    }
    assert!(f.fs.file(&path).is_some(), "the file survived the agent");
    assert_eq!(f.list.snapshot().len(), 2);

    // Taking a user-owned file off the list is only a list edit: allowed.
    f.run(
        f.service
            .execute(&add_file("/work/a.yaml"), Initiator::Agent),
    )
    .unwrap();
    let remove_owned = Command::KubeconfigRemoveSource {
        source: KubeconfigSourceRef::File {
            path: "/work/a.yaml".into(),
        },
    };
    f.run(f.service.execute(&remove_owned, Initiator::Agent))
        .unwrap();

    // The user can delete it.
    f.run(f.service.execute(&remove, Initiator::Ui)).unwrap();
    assert!(f.fs.file(&path).is_none());
}

#[test]
fn a_command_that_is_not_a_kubeconfig_command_is_unsupported() {
    let f = Fixture::with_defaults();
    let error = f
        .run(f.service.execute(&Command::AppQuit, Initiator::Ui))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}
