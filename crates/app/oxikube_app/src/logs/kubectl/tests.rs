use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use oxikube_ports::{LogOptions, LogSince};

use super::*;

fn tail(target: TailTarget, options: LogOptions) -> KubectlTail {
    KubectlTail {
        context: "kind-dev".into(),
        namespace: "shop".into(),
        target,
        options,
        timestamps: false,
        max_log_requests: 20,
    }
}

fn pod(name: &str) -> TailTarget {
    TailTarget::Pod(name.into())
}

fn argv(tail: &KubectlTail) -> Vec<String> {
    tail.argv().expect("a valid command")
}

fn follow() -> LogOptions {
    LogOptions {
        follow: true,
        timestamps: true,
        tail_lines: Some(1_000),
        ..LogOptions::default()
    }
}

#[test]
fn a_pod_is_followed_with_its_context_and_namespace() {
    assert_eq!(
        argv(&tail(pod("web-0"), follow())),
        [
            "logs",
            "-f",
            "--context=kind-dev",
            "--namespace=shop",
            "--tail=1000",
            "web-0"
        ]
    );
}

#[test]
fn the_views_options_become_flags() {
    let options = LogOptions {
        container: Some("sidecar".into()),
        since: Some(LogSince::Seconds(1_800)),
        tail_lines: None,
        ..follow()
    };
    let mut request = tail(pod("web-0"), options);
    request.timestamps = true;
    assert_eq!(
        argv(&request),
        [
            "logs",
            "-f",
            "--context=kind-dev",
            "--namespace=shop",
            "--container=sidecar",
            "--timestamps",
            "--since=1800s",
            "web-0"
        ]
    );
}

#[test]
fn the_previous_instance_and_the_head_do_not_follow() {
    let previous = LogOptions {
        previous: true,
        follow: false,
        tail_lines: Some(50),
        ..LogOptions::default()
    };
    let args = argv(&tail(pod("web-0"), previous));
    assert!(!args.contains(&"-f".to_owned()), "{args:?}");
    assert!(args.contains(&"--previous".to_owned()));
    assert!(args.contains(&"--tail=50".to_owned()));

    let head = LogOptions {
        limit_bytes: Some(1 << 20),
        ..LogOptions::default()
    };
    let args = argv(&tail(pod("web-0"), head));
    assert!(!args.contains(&"-f".to_owned()));
    assert!(args.contains(&"--limit-bytes=1048576".to_owned()));
}

#[test]
fn since_a_time_uses_since_time() {
    let time: jiff::Timestamp = "2026-10-07T01:02:03Z".parse().unwrap();
    let options = LogOptions {
        since: Some(LogSince::Time(time)),
        ..follow()
    };
    let args = argv(&tail(pod("web-0"), options));
    assert!(
        args.contains(&"--since-time=2026-10-07T01:02:03Z".to_owned()),
        "{args:?}"
    );
}

#[test]
fn a_selector_prefixes_every_pod_and_reads_every_container() {
    let request = tail(
        TailTarget::Selector("app=web,tier in (a,b)".into()),
        follow(),
    );
    assert_eq!(
        argv(&request),
        [
            "logs",
            "-f",
            "--context=kind-dev",
            "--namespace=shop",
            "--all-containers=true",
            "--tail=1000",
            "--selector=app=web,tier in (a,b)",
            "--prefix",
            "--max-log-requests=20"
        ]
    );
    // A named container is read alone.
    let options = LogOptions {
        container: Some("app".into()),
        ..follow()
    };
    let args = argv(&tail(TailTarget::Selector("app=web".into()), options));
    assert!(args.contains(&"--container=app".to_owned()));
    assert!(!args.contains(&"--all-containers=true".to_owned()));
}

#[test]
fn a_selector_read_by_time_asks_for_every_line_not_kubectls_ten() {
    let options = LogOptions {
        since: Some(LogSince::Seconds(60)),
        tail_lines: None,
        ..follow()
    };
    let args = argv(&tail(TailTarget::Selector("app=web".into()), options));
    assert!(args.contains(&"--tail=-1".to_owned()), "{args:?}");
}

#[test]
fn odd_names_stay_one_argument_each() {
    // There is no shell: spaces, quotes and substitutions are just characters.
    let nasty = "a b'c\"d$(touch x);e`f`\n";
    let mut request = tail(pod(nasty), follow());
    request.context = "arn:aws:eks:eu-west-1:1:cluster/my cluster".into();
    request.namespace = "ns; rm -rf /".into();
    request.options.container = Some("--all-namespaces".into());
    let args = argv(&request);
    assert_eq!(args.last().map(String::as_str), Some(nasty));
    assert!(args.contains(&"--context=arn:aws:eks:eu-west-1:1:cluster/my cluster".to_owned()));
    assert!(args.contains(&"--namespace=ns; rm -rf /".to_owned()));
    assert!(
        args.contains(&"--container=--all-namespaces".to_owned()),
        "a value that looks like a flag is glued to its flag"
    );
    assert_eq!(args.iter().filter(|a| a.contains("touch")).count(), 1);
}

#[test]
fn what_cannot_be_a_command_is_refused() {
    let flag_like = tail(pod("--all-namespaces"), follow());
    assert!(flag_like.argv().is_err(), "a pod name cannot be a flag");
    assert!(tail(pod(" "), follow()).argv().is_err());
    assert!(
        tail(TailTarget::Selector(String::new()), follow())
            .argv()
            .is_err()
    );
    let mut no_context = tail(pod("web-0"), follow());
    no_context.context.clear();
    assert!(no_context.argv().is_err());
    let mut no_namespace = tail(pod("web-0"), follow());
    no_namespace.namespace = " ".into();
    assert!(no_namespace.argv().is_err());
}

#[test]
fn a_command_line_carries_nothing_secret() {
    // The argv names what is read and nothing else: no token, no kubeconfig path, no env value.
    let args = argv(&tail(pod("web-0"), follow())).join(" ");
    for secret in ["token", "KUBECONFIG", "password", "client-key"] {
        assert!(!args.contains(secret), "{secret} in {args}");
    }
}

// --- looking for kubectl ---------------------------------------------------------------------

#[cfg(unix)]
mod lookup {
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("oxikube-kubectl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn the_first_executable_kubectl_in_the_folders_wins() {
        let (a, b, c) = (scratch("a"), scratch("b"), scratch("c"));
        put(&a, "kubectl", 0o644); // present but not executable
        let wanted = put(&b, "kubectl", 0o755);
        put(&c, "kubectl", 0o755);
        let lookup = PathLookup::in_dirs([a.clone(), b.clone(), c.clone()]);
        assert_eq!(lookup.find(), Some(wanted));
        for dir in [a, b, c] {
            std::fs::remove_dir_all(dir).ok();
        }
    }

    #[test]
    fn a_folder_with_other_programs_or_a_directory_named_kubectl_finds_nothing() {
        let dir = scratch("none");
        put(&dir, "kubectl-old", 0o755);
        std::fs::create_dir(dir.join("kubectl")).unwrap();
        assert_eq!(PathLookup::in_dirs([dir.clone()]).find(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn relative_folders_in_path_are_ignored() {
        // `PATH=.:bin` must not run a kubectl dropped in the working directory.
        let dir = scratch("rel");
        put(&dir, "kubectl", 0o755);
        let lookup = PathLookup::in_dirs([PathBuf::from("."), PathBuf::from("")]);
        assert_eq!(lookup.find(), None);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_path_variable_comes_first_then_the_usual_folders() {
        let dir = scratch("var");
        let wanted = put(&dir, "kubectl", 0o755);
        let lookup = PathLookup::from_path_var(Some(dir.clone().into_os_string()));
        assert_eq!(lookup.find(), Some(wanted));
        std::fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn the_cached_answer_is_unknown_until_refreshed_and_follows_the_lookup() {
    let installed = Arc::new(AtomicBool::new(false));
    let flag = installed.clone();
    let kubectl = Kubectl::new(move || {
        flag.load(Ordering::SeqCst)
            .then(|| PathBuf::from("/opt/bin/kubectl"))
    });
    assert!(!kubectl.is_available(), "nothing is known before a refresh");
    assert_eq!(kubectl.path(), None);

    assert!(!kubectl.refresh());
    assert!(!kubectl.is_available());

    installed.store(true, Ordering::SeqCst);
    assert!(
        !kubectl.is_available(),
        "the cache is not stale-checked on read"
    );
    let clone = kubectl.clone();
    assert!(kubectl.refresh());
    assert_eq!(
        clone.path(),
        Some(PathBuf::from("/opt/bin/kubectl")),
        "clones share it"
    );

    installed.store(false, Ordering::SeqCst);
    kubectl.refresh();
    assert!(!clone.is_available(), "an uninstalled kubectl is forgotten");
}
