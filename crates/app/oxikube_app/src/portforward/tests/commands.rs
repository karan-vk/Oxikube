//! `portforward::*` handlers and the `service::OpenInBrowser` opener.

use oxikube_domain::ErrorKind;
use oxikube_domain::command::{self, Command};
use oxikube_domain::portforward::ForwardPort;
use serde_json::json;

use super::{Rig, pod, service, slice, target};
use crate::command_bus::{CommandRegistry, HandlerContext, RegisterError};
use crate::network::ServicePortOpener;
use crate::portforward::commands::{COMMANDS, register_commands, run};

#[tokio::test]
async fn the_commands_start_stop_restart_and_remove() {
    let rig = Rig::new();
    rig.backend.push("Pod", vec![pod("api-0", "Running")]);
    let out = run(
        &rig.manager,
        Command::PortForwardStart {
            target: target("Pod", "api-0"),
            remote_port: ForwardPort::Number(80),
            local_port: None,
            bind: None,
        },
    )
    .await
    .expect("start");
    assert!(out.message.unwrap().starts_with("Forwarding 127.0.0.1:"));
    let id = out.data.unwrap()["id"].as_u64().unwrap();

    for command in [
        Command::PortForwardStop { id },
        Command::PortForwardRestart { id },
        Command::PortForwardRemove { id },
    ] {
        run(&rig.manager, command).await.expect("known id");
    }
    let err = run(&rig.manager, Command::PortForwardStop { id })
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn a_refused_start_says_why_and_what_to_do() {
    let rig = Rig::new();
    *rig.backend.read_only.lock() = true;
    let err = run(
        &rig.manager,
        Command::PortForwardStart {
            target: target("Pod", "api-0"),
            remote_port: ForwardPort::Number(80),
            local_port: None,
            bind: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
    assert!(
        err.message().contains("allowed_in_readonly"),
        "{}",
        err.message()
    );
}

#[test]
fn every_command_is_registered_with_its_tool() {
    let mut registry = CommandRegistry::new();
    register_commands(&mut registry, Rig::new().manager).expect("registered");
    for id in COMMANDS {
        let meta = *command::lookup(id).unwrap();
        assert!(!meta.mutating, "{id} changes no cluster");
        assert!(id.tool_name().starts_with("k8s.portforward_"), "{id}");
        let again = registry.register(meta, |_: Command, _: HandlerContext| async {
            Ok(crate::command_bus::CommandOutput::default())
        });
        assert!(
            matches!(again, Err(RegisterError::Duplicate { .. })),
            "{id} is taken"
        );
    }
}

#[tokio::test]
async fn open_in_browser_forwards_a_service_port_and_reuses_it() {
    let rig = Rig::new();
    rig.backend.push(
        "Service",
        vec![service(
            "web",
            json!([{"name": "http", "port": 80, "targetPort": 8080}]),
        )],
    );
    rig.backend.push(
        "EndpointSlice",
        vec![slice(
            "web-x",
            "web",
            json!([{"name": "http", "port": 8080}]),
            &[("web-a", true)],
        )],
    );
    let opened = rig
        .manager
        .open_service_port(target("Service", "web"), None)
        .await
        .expect("opened");
    assert_eq!(opened.service_port, 80);
    assert!(
        opened.url.starts_with("http://localhost:"),
        "{}",
        opened.url
    );
    let again = rig
        .manager
        .open_service_port(target("Service", "web"), Some(80))
        .await
        .expect("opened");
    assert_eq!(again.url, opened.url, "the active forward is reused");
    assert_eq!(rig.manager.list().len(), 1);

    let err = rig
        .manager
        .open_service_port(target("Service", "web"), Some(443))
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}
