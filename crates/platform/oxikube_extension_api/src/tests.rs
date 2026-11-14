use std::collections::BTreeSet;
use std::path::Path;

use wit_parser::{Resolve, WorldItem, WorldKey};

use super::{Command, Component, Extension, register_extension, wit::Guest};

/// The world as `wit-parser` resolves it from `wit/since_v0.1.0`.
fn resolve_world() -> (Resolve, wit_parser::WorldId) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("wit/since_v0.1.0");
    let mut resolve = Resolve::default();
    let (package, _files) = resolve.push_dir(&dir).expect("every WIT file parses");
    let world = resolve
        .select_world(&[package], Some("extension"))
        .expect("world `extension`");
    (resolve, world)
}

fn names(resolve: &Resolve, keys: &[&WorldKey]) -> BTreeSet<String> {
    keys.iter().map(|key| resolve.name_world_key(key)).collect()
}

#[test]
fn world_is_versioned_and_imports_every_host_interface() {
    let (resolve, world) = resolve_world();
    let world = &resolve.worlds[world];
    let package = &resolve.packages[world.package.expect("world belongs to a package")];
    assert_eq!(package.name.to_string(), "oxikube:extension@0.1.0");

    let imports: Vec<&WorldKey> = world
        .imports
        .iter()
        .filter(|(_, item)| matches!(item, WorldItem::Interface { .. }))
        .map(|(key, _)| key)
        .collect();
    let expected: BTreeSet<String> = [
        "common",
        "http-client",
        "key-value-store",
        "kube-info",
        "platform",
        "process",
        "settings",
    ]
    .iter()
    .map(|name| format!("oxikube:extension/{name}@0.1.0"))
    .collect();
    assert_eq!(names(&resolve, &imports), expected);
}

#[test]
fn world_exports_only_the_three_entry_points_and_no_ui() {
    let (resolve, world) = resolve_world();
    let exports: Vec<&WorldKey> = resolve.worlds[world].exports.keys().collect();
    let expected: BTreeSet<String> = ["context-server-command", "init-extension", "run-command"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(names(&resolve, &exports), expected);
}

struct Echo;

impl Extension for Echo {
    fn new() -> Self {
        Echo
    }

    fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
        if command_id == "echo" && args == ["hi"] {
            Ok(())
        } else {
            Err(command_id.to_string())
        }
    }
}

#[test]
fn exports_fail_until_an_extension_is_registered_then_forward_to_it() {
    // Each test runs on its own thread, so the thread-local slot starts empty here.
    assert!(Component::run_command("echo".into(), vec![]).is_err());

    register_extension(|| Box::new(Echo::new()));

    assert_eq!(
        Component::run_command("echo".into(), vec!["hi".into()]),
        Ok(())
    );
    assert_eq!(
        Component::run_command("other".into(), vec![]),
        Err("other".to_string())
    );
    // Echo keeps the default `context_server_command`, which names the unknown id.
    let err: Result<Command, String> = Component::context_server_command("mcp".into());
    assert_eq!(
        err.map(|c| c.command),
        Err("unknown context server `mcp`".to_string())
    );
}
