//! Listing: the scripts that run, the fallbacks, and the errors.

use oxikube_domain::ErrorKind;
use oxikube_ports::ExitStatus;
use oxikube_testkit::ExecScript;

use super::Fixture;
use crate::files::{FileKind, ListingSource, NO_SHELL_TOOLS};
use crate::testing::pod;

const GNU: &str = "total 8\n\
drwxr-xr-x 2 root root 4096 2024-03-05 12:34:56.000000000 +0000 .\n\
drwxr-xr-x 1 root root 4096 2024-03-05 12:34:56.000000000 +0000 ..\n\
-rw-r--r-- 1 root root 12 2024-03-05 12:34:56.000000000 +0000 hello world.txt\n\
drwxr-xr-x 2 root root 4096 2024-03-05 12:34:56.000000000 +0000 sub\n";

#[tokio::test]
async fn ls_runs_in_the_c_locale_without_a_tty_and_its_output_is_parsed() {
    let f = Fixture::new(false);
    f.script(ExecScript::new().stdout(GNU));
    let listing = f
        .service
        .list(&pod("a", "web-0"), Some("app"), "/etc")
        .await
        .expect("listing");
    assert_eq!(listing.source, ListingSource::Ls);
    let names: Vec<_> = listing.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["hello world.txt", "sub"]);
    assert_eq!(listing.entries[1].kind, FileKind::Directory);
    assert_eq!(listing.entries[0].size, Some(12));

    let command = &f.commands()[0];
    assert_eq!(command[..2], ["sh", "-c"]);
    assert!(command[2].contains("LC_ALL=C"), "{command:?}");
    assert!(command[2].contains("--full-time"), "{command:?}");
    assert_eq!(
        command[4], "/etc/",
        "a trailing slash lists a linked directory"
    );
    let calls = f.exec.recorded_calls();
    let oxikube_testkit::ExecStreamCall::Exec { options, .. } = &calls[0] else {
        panic!("an exec");
    };
    assert!(!options.tty, "a TTY would rewrite the output");
    assert_eq!(options.container.as_deref(), Some("app"));
}

#[tokio::test]
async fn a_container_without_ls_is_listed_with_find() {
    let f = Fixture::new(false);
    f.script(
        ExecScript::new()
            .exit(Ok(ExitStatus::with_code(127)))
            .stderr("sh: ls: not found"),
    );
    f.script(ExecScript::new().stdout("d /srv/data\nf /srv/index.html\n"));
    let listing = f
        .service
        .list(&pod("a", "web-0"), None, "/srv")
        .await
        .expect("listing");
    assert_eq!(listing.source, ListingSource::Find);
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.entries[0].kind, FileKind::Directory);
    assert!(f.commands()[1][2].contains("find"));
}

#[tokio::test]
async fn a_container_with_no_shell_tools_gets_the_clear_message_not_an_error_loop() {
    // Neither ls nor find.
    let f = Fixture::new(false);
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    f.script(ExecScript::new().exit(Ok(ExitStatus::with_code(127))));
    let error = f
        .service
        .list(&pod("a", "web-0"), None, "/")
        .await
        .expect_err("nothing to list with");
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(error.message(), NO_SHELL_TOOLS);
    assert!(error.message().contains("debug container"));

    // No sh at all: the runtime refuses the program.
    let f = Fixture::new(false);
    f.exec
        .script()
        .exec
        .push_err(oxikube_domain::OxiError::conflict(
            "OCI runtime exec failed: exec failed: executable file not found in $PATH",
        ));
    let error = f
        .service
        .list(&pod("a", "web-0"), None, "/")
        .await
        .expect_err("no sh");
    assert_eq!(error.message(), NO_SHELL_TOOLS);
}

#[tokio::test]
async fn failures_are_typed_from_what_ls_said() {
    for (said, kind) in [
        (
            "ls: cannot access '/x/': No such file or directory",
            ErrorKind::NotFound,
        ),
        (
            "ls: cannot open directory '/root/': Permission denied",
            ErrorKind::Forbidden,
        ),
        ("ls: /etc/passwd/: Not a directory", ErrorKind::Validation),
        ("ls: something odd", ErrorKind::Conflict),
    ] {
        let f = Fixture::new(false);
        f.script(
            ExecScript::new()
                .stderr(said)
                .exit(Ok(ExitStatus::with_code(2))),
        );
        let error = f
            .service
            .list(&pod("a", "web-0"), None, "/x")
            .await
            .expect_err(said);
        assert_eq!(error.kind(), kind, "{said}: {error}");
    }
}

#[tokio::test]
async fn a_partial_listing_with_permission_errors_is_still_shown() {
    let f = Fixture::new(false);
    f.script(
        ExecScript::new()
            .stdout("total 4\n-rw-r--r-- 1 root root 5 Mar  5 12:34 ok.txt\n-????????? ? ? ? ? ? locked\n")
            .stderr("ls: cannot access '/d/locked': Permission denied")
            .exit(Ok(ExitStatus::with_code(2))),
    );
    let listing = f
        .service
        .list(&pod("a", "web-0"), None, "/d")
        .await
        .expect("partial");
    assert_eq!(listing.entries.len(), 2);
    assert_eq!(listing.unreadable, 1);
}

#[tokio::test]
async fn the_last_directory_is_remembered_per_pod_and_container_in_memory() {
    let f = Fixture::new(false);
    let web = pod("a", "web-0");
    assert_eq!(f.service.last_path(&web, "app"), None);
    f.service.remember_path(&web, "app", "/var/log");
    f.service.remember_path(&web, "sidecar", "/");
    assert_eq!(
        f.service.last_path(&web, "app").as_deref(),
        Some("/var/log")
    );
    assert_eq!(f.service.last_path(&web, "sidecar").as_deref(), Some("/"));
    assert_eq!(f.service.last_path(&pod("a", "web-1"), "app"), None);
}
