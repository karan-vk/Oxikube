//! Parsing and planning `terminal::OpenLink` targets.

use std::path::PathBuf;

use super::*;

#[test]
fn browser_urls_parse() {
    for url in [
        "https://kubernetes.io/docs/",
        "http://localhost:8080/x?y=1",
        "mailto:ops@example.com",
    ] {
        assert!(
            matches!(LinkTarget::parse(url), Ok(LinkTarget::Url(_))),
            "{url}"
        );
    }
}

#[test]
fn other_schemes_and_relative_paths_are_refused() {
    for bad in [
        "javascript:alert(1)",
        "ssh://host",
        "vscode://file/x",
        "file://remote-host/etc/passwd",
        "src/main.rs",
        "",
    ] {
        assert!(LinkTarget::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn paths_and_file_urls_keep_their_position_apart() {
    assert_eq!(
        LinkTarget::parse("/src/main.rs:12:3").unwrap(),
        LinkTarget::Path {
            path: "/src/main.rs".into(),
            line: Some(12),
            column: Some(3),
        }
    );
    assert_eq!(
        LinkTarget::parse("/etc/hosts:7").unwrap(),
        LinkTarget::Path {
            path: "/etc/hosts".into(),
            line: Some(7),
            column: None,
        }
    );
    assert_eq!(
        LinkTarget::parse("/var/log/a:b").unwrap(),
        LinkTarget::Path {
            path: "/var/log/a:b".into(),
            line: None,
            column: None,
        }
    );
    assert_eq!(
        LinkTarget::parse("file:///tmp/report%20one.txt").unwrap(),
        LinkTarget::Path {
            path: "/tmp/report one.txt".into(),
            line: None,
            column: None,
        }
    );
}

fn path_target(path: PathBuf) -> LinkTarget {
    LinkTarget::Path {
        path,
        line: Some(1),
        column: None,
    }
}

#[test]
fn files_open_but_directories_and_executables_are_only_revealed() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "x").unwrap();
    assert_eq!(
        plan(path_target(file.clone())).unwrap(),
        LinkAction::OpenFile(file)
    );
    assert_eq!(
        plan(path_target(dir.path().to_path_buf())).unwrap(),
        LinkAction::Reveal(dir.path().to_path_buf())
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let script = dir.path().join("run.command");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            plan(path_target(script.clone())).unwrap(),
            LinkAction::Reveal(script)
        );
    }
    let missing = dir.path().join("missing.rs");
    assert!(plan(path_target(missing)).is_err());
    assert_eq!(
        plan(LinkTarget::Url("https://x.y/".into())).unwrap(),
        LinkAction::Browse("https://x.y/".into())
    );
}
