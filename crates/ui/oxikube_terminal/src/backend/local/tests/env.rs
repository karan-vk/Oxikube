//! The shell's environment: cluster variables set, `PATH` and the rest untouched.

use std::ffi::OsStr;

use oxikube_domain::ids::ContextName;

use super::super::options::build_command;
use super::*;

fn vars() -> Vec<(&'static str, String)> {
    vec![
        (
            "KUBECONFIG",
            "/run/user/1/oxikube-term-9/kubeconfig-0".into(),
        ),
        ("KUBE_CONTEXT", "prod".into()),
        ("OXIKUBE_NAMESPACE", "payments".into()),
    ]
}

#[test]
fn the_command_carries_the_cluster_variables_and_leaves_path_alone() {
    let options = LocalPtyOptions {
        shell: Some("/bin/zsh".into()),
        args: vec!["-l".into()],
        cwd: Some("/work".into()),
        ..LocalPtyOptions::default()
    };
    let command = build_command(&options, &vars());
    assert_eq!(command.get_argv()[0], OsStr::new("/bin/zsh"));
    assert_eq!(command.get_argv()[1], OsStr::new("-l"));
    assert_eq!(
        command.get_cwd().map(|c| c.as_os_str()),
        Some(OsStr::new("/work"))
    );
    for (key, value) in vars() {
        assert_eq!(command.get_env(key), Some(OsStr::new(&value)), "{key}");
    }
    // PATH is the process's own value, byte for byte.
    assert_eq!(
        command.get_env("PATH").map(OsStr::to_owned),
        std::env::var_os("PATH")
    );
    assert_eq!(command.get_env("TERM"), Some(OsStr::new("xterm-256color")));
}

#[test]
fn a_plain_terminal_sets_no_cluster_variables() {
    let command = build_command(&LocalPtyOptions::default(), &[]);
    // Inherited from the test process, if present there; never ours.
    for key in ["KUBE_CONTEXT", "OXIKUBE_NAMESPACE"] {
        assert_eq!(
            command.get_env(key).map(OsStr::to_owned),
            std::env::var_os(key)
        );
    }
}

#[test]
fn a_cluster_terminal_for_an_unknown_context_fails_before_forking() {
    let cluster = ClusterEnv::new(ContextName::new("ghost"), Vec::new());
    let error = LocalPty::spawn(sh("true").for_cluster(cluster))
        .err()
        .unwrap();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::NotFound);
}

#[test]
fn a_shell_that_does_not_exist_is_a_validation_error() {
    let options = LocalPtyOptions {
        shell: Some("/definitely/not/a/shell".into()),
        ..LocalPtyOptions::default()
    };
    let error = LocalPty::spawn(options).err().unwrap();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
}

#[test]
fn a_zero_size_is_rejected() {
    let options = sh("true").with_size(TerminalSize::new(0, 24));
    let error = LocalPty::spawn(options).err().unwrap();
    assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
}
