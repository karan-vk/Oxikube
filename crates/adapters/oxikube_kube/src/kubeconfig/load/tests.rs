use std::ffi::OsString;
use std::fs;

use oxikube_domain::ErrorKind;
use proptest::prelude::*;
use tempfile::TempDir;

use super::*;
use crate::kubeconfig::Severity;

const SECRET: &str = "s3cr3t-token-do-not-leak";

/// A kubeconfig defining one cluster, user and context per name; `server` tags the cluster so
/// tests can tell which file an entry came from.
fn config_yaml(names: &[&str], server: &str, current: Option<&str>) -> String {
    let mut out = String::from("apiVersion: v1\nkind: Config\n");
    if let Some(current) = current {
        out.push_str(&format!("current-context: {current}\n"));
    }
    out.push_str("clusters:\n");
    for name in names {
        out.push_str(&format!(
            "- name: {name}\n  cluster:\n    server: https://{name}.{server}\n"
        ));
    }
    out.push_str("users:\n");
    for name in names {
        out.push_str(&format!("- name: {name}\n  user:\n    token: {SECRET}\n"));
    }
    out.push_str("contexts:\n");
    for name in names {
        out.push_str(&format!(
            "- name: {name}\n  context:\n    cluster: {name}\n    user: {name}\n"
        ));
    }
    out
}

fn write(dir: &TempDir, name: &str, text: &str) -> PathBuf {
    let path = dir.path().join(name);
    fs::write(&path, text).expect("write fixture");
    path
}

fn load(paths: &[PathBuf]) -> LoadedKubeconfig {
    load_kubeconfig_from_paths_blocking(paths, Strictness::Tolerant).expect("tolerant never fails")
}

fn names(loaded: &LoadedKubeconfig) -> Vec<String> {
    loaded.context_names().map(|c| c.to_string()).collect()
}

fn server_of(loaded: &LoadedKubeconfig, cluster: &str) -> String {
    loaded
        .merged
        .clusters
        .iter()
        .find(|c| c.name == cluster)
        .and_then(|c| c.cluster.as_ref())
        .and_then(|c| c.server.clone())
        .expect("cluster with server")
}

#[test]
fn blank_files_are_skipped() {
    let dir = TempDir::new().unwrap();
    let empty = write(&dir, "empty", "");
    let spaces = write(&dir, "spaces", "  \n\t\n\n");
    let comment = write(&dir, "comment", "# nothing here\n");
    let good = write(&dir, "good", &config_yaml(&["a"], "x", None));

    let loaded = load(&[empty.clone(), spaces.clone(), comment.clone(), good]);

    assert_eq!(names(&loaded), ["a"]);
    for path in [empty, spaces, comment] {
        assert!(
            loaded
                .diagnostics
                .contains(&Diagnostic::BlankFile { path: path.clone() }),
            "{path:?}"
        );
    }
    assert!(
        loaded
            .diagnostics
            .iter()
            .all(|d| d.severity() == Severity::Info)
    );
}

#[test]
fn missing_path_in_the_middle_is_skipped() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));
    let missing = dir.path().join("does-not-exist");
    let b = write(&dir, "b", &config_yaml(&["b"], "x", None));

    let loaded = load(&[a, missing.clone(), b]);

    assert_eq!(names(&loaded), ["a", "b"]);
    assert_eq!(
        loaded.diagnostics,
        [Diagnostic::MissingFile {
            path: missing.clone()
        }]
    );
    let statuses: Vec<_> = loaded.sources.iter().map(|s| s.status).collect();
    assert_eq!(
        statuses,
        [
            SourceStatus::Loaded,
            SourceStatus::Missing,
            SourceStatus::Loaded
        ]
    );
    // The missing path stays listed so a watcher can observe it appearing.
    assert_eq!(loaded.sources[1].path, missing);
}

#[test]
fn corrupt_file_between_good_ones_is_skipped_without_leaking_contents() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));
    let corrupt = write(
        &dir,
        "corrupt",
        &format!("clusters: [\n  token: {SECRET}\n  : : :\n"),
    );
    let b = write(&dir, "b", &config_yaml(&["b"], "x", None));

    let loaded = load(&[a, corrupt.clone(), b]);

    assert_eq!(names(&loaded), ["a", "b"]);
    assert_eq!(
        loaded.diagnostics,
        [Diagnostic::Unparsable { path: corrupt }]
    );
    assert_eq!(loaded.diagnostics[0].severity(), Severity::Warning);
    let text = format!("{:?} {}", loaded.diagnostics, loaded.diagnostics[0]);
    assert!(text.contains("corrupt"), "names the file: {text}");
    assert!(!text.contains(SECRET), "leaked file contents: {text}");
}

#[test]
fn directory_in_the_list_is_unreadable_not_fatal() {
    let dir = TempDir::new().unwrap();
    let good = write(&dir, "good", &config_yaml(&["a"], "x", None));

    let loaded = load(&[dir.path().to_path_buf(), good]);

    assert_eq!(names(&loaded), ["a"]);
    assert!(matches!(
        loaded.diagnostics.as_slice(),
        [Diagnostic::Unreadable { .. }]
    ));
}

#[test]
fn duplicate_context_names_first_file_wins() {
    let dir = TempDir::new().unwrap();
    let first = write(
        &dir,
        "first",
        &config_yaml(&["shared", "only1"], "one", None),
    );
    let second = write(
        &dir,
        "second",
        &config_yaml(&["shared", "only2"], "two", None),
    );

    let loaded = load(&[first.clone(), second.clone()]);

    // One `shared`, from the first file, with the first file's cluster and user.
    assert_eq!(names(&loaded), ["shared", "only1", "only2"]);
    assert_eq!(server_of(&loaded, "shared"), "https://shared.one");
    assert_eq!(loaded.origin(&"shared".into()), Some(first.as_path()));
    assert_eq!(loaded.origin(&"only2".into()), Some(second.as_path()));
    // Both origins are recorded for the UI.
    assert_eq!(
        loaded.diagnostics,
        [Diagnostic::DuplicateContext {
            context: "shared".into(),
            winner: first,
            shadowed: second,
        }]
    );
    // The shadowing file still lists the name, so S02 can diff it.
    assert!(loaded.sources[1].contexts.contains(&"shared".into()));
}

#[test]
fn reversing_the_order_flips_the_winner() {
    let dir = TempDir::new().unwrap();
    let one = write(&dir, "one", &config_yaml(&["shared"], "one", None));
    let two = write(&dir, "two", &config_yaml(&["shared"], "two", None));

    let loaded = load(&[two.clone(), one]);

    assert_eq!(server_of(&loaded, "shared"), "https://shared.two");
    assert_eq!(loaded.origin(&"shared".into()), Some(two.as_path()));
}

#[test]
fn name_repeated_inside_one_file_is_kept_once() {
    let dir = TempDir::new().unwrap();
    // A repeat within one document is the case `Kubeconfig::merge` does not filter.
    let doubled = config_yaml(&["dup"], "first", None).replace(
        "contexts:\n",
        "contexts:\n- name: dup\n  context:\n    cluster: dup\n    user: dup\n",
    );
    let path = write(&dir, "kc", &doubled);

    let loaded = load(std::slice::from_ref(&path));

    assert_eq!(loaded.merged.contexts.len(), 1);
    assert_eq!(
        loaded.diagnostics,
        [Diagnostic::DuplicateContext {
            context: "dup".into(),
            winner: path.clone(),
            shadowed: path,
        }]
    );
    assert!(loaded.diagnostics[0].to_string().contains("more than once"));
}

#[test]
fn current_context_and_kind_come_from_the_first_file() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", Some("a")));
    let b = write(&dir, "b", &config_yaml(&["b"], "x", Some("b")));

    let loaded = load(&[a, b]);

    assert_eq!(loaded.merged.current_context.as_deref(), Some("a"));
}

#[test]
fn merge_order_follows_the_listed_order() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a1", "a2"], "x", None));
    let b = write(&dir, "b", &config_yaml(&["b1"], "x", None));
    let c = write(&dir, "c", &config_yaml(&["c1"], "x", None));

    assert_eq!(
        names(&load(&[a.clone(), b.clone(), c.clone()])),
        ["a1", "a2", "b1", "c1"]
    );
    assert_eq!(names(&load(&[c, a, b])), ["c1", "a1", "a2", "b1"]);
}

#[test]
fn every_context_maps_to_its_file() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a1", "a2"], "x", None));
    let b = write(&dir, "b", &config_yaml(&["b1"], "x", None));

    let loaded = load(&[a.clone(), b.clone()]);

    assert_eq!(loaded.origins.len(), 3);
    assert_eq!(loaded.origin(&"a1".into()), Some(a.as_path()));
    assert_eq!(loaded.origin(&"a2".into()), Some(a.as_path()));
    assert_eq!(loaded.origin(&"b1".into()), Some(b.as_path()));
    assert_eq!(loaded.origin(&"nope".into()), None);
}

#[test]
fn cluster_id_depends_on_origin_file_and_context() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["x"], "one", None));
    let b = write(&dir, "b", &config_yaml(&["x"], "two", None));

    let from_a = load(std::slice::from_ref(&a))
        .cluster_id(&"x".into())
        .unwrap();
    let from_b = load(&[b]).cluster_id(&"x".into()).unwrap();
    let again = load(std::slice::from_ref(&a))
        .cluster_id(&"x".into())
        .unwrap();

    assert_eq!(from_a, again);
    assert_ne!(from_a, from_b);
    assert_eq!(load(&[]).cluster_id(&"x".into()), None);
}

#[test]
fn relative_cert_paths_resolve_against_each_files_directory() {
    let root = TempDir::new().unwrap();
    let dir_a = root.path().join("a");
    let dir_b = root.path().join("b");
    for dir in [&dir_a, &dir_b] {
        fs::create_dir(dir).unwrap();
        for file in ["ca.pem", "client.pem", "client.key"] {
            fs::write(dir.join(file), "pem").unwrap();
        }
    }
    let template = |name: &str| {
        format!(
            "apiVersion: v1\nkind: Config\n\
             clusters:\n- name: {name}\n  cluster:\n    server: https://{name}.example\n    certificate-authority: ca.pem\n\
             users:\n- name: {name}\n  user:\n    client-certificate: client.pem\n    client-key: ./client.key\n\
             contexts:\n- name: {name}\n  context:\n    cluster: {name}\n    user: {name}\n"
        )
    };
    let file_a = dir_a.join("config");
    let file_b = dir_b.join("config");
    fs::write(&file_a, template("a")).unwrap();
    fs::write(&file_b, template("b")).unwrap();

    let loaded = load(&[file_a, file_b]);

    for (name, dir) in [("a", &dir_a), ("b", &dir_b)] {
        let cluster = loaded
            .merged
            .clusters
            .iter()
            .find(|c| c.name == name)
            .and_then(|c| c.cluster.as_ref())
            .unwrap();
        let user = loaded
            .merged
            .auth_infos
            .iter()
            .find(|u| u.name == name)
            .and_then(|u| u.auth_info.as_ref())
            .unwrap();
        let ca = PathBuf::from(cluster.certificate_authority.clone().unwrap());
        let cert = PathBuf::from(user.client_certificate.clone().unwrap());
        let key = PathBuf::from(user.client_key.clone().unwrap());
        assert_eq!(ca, dir.join("ca.pem"), "ca of {name}");
        assert_eq!(cert, dir.join("client.pem"), "cert of {name}");
        assert_eq!(key, dir.join(".").join("client.key"), "key of {name}");
        // Resolved paths point at the right file, whichever merged position they ended up in.
        for path in [&ca, &cert, &key] {
            assert!(path.is_absolute() && path.exists(), "{path:?}");
        }
    }
}

#[test]
fn relative_listed_path_still_yields_absolute_cert_paths() {
    // Build the relative path from the real cwd rather than changing it.
    let cwd = std::env::current_dir().unwrap();
    let dir = TempDir::new_in(&cwd).unwrap();
    fs::write(dir.path().join("ca.pem"), "pem").unwrap();
    let text = "clusters:\n- name: a\n  cluster:\n    server: https://a\n    certificate-authority: ca.pem\n\
                contexts:\n- name: a\n  context:\n    cluster: a\n";
    fs::write(dir.path().join("config"), text).unwrap();
    let relative = dir.path().strip_prefix(&cwd).unwrap().join("config");
    assert!(relative.is_relative());

    let loaded = load(std::slice::from_ref(&relative));

    let cluster = loaded.merged.clusters[0].cluster.as_ref().unwrap();
    let ca = PathBuf::from(cluster.certificate_authority.clone().unwrap());
    assert!(ca.is_absolute(), "{ca:?}");
    assert!(ca.exists(), "{ca:?}");
    // The listed path is kept for sources and origins.
    assert_eq!(loaded.sources[0].path, relative);
    assert_eq!(loaded.origin(&"a".into()), Some(relative.as_path()));
}

#[test]
fn absolute_cert_paths_are_left_alone() {
    let dir = TempDir::new().unwrap();
    let ca = dir.path().join("elsewhere-ca.pem");
    fs::write(&ca, "pem").unwrap();
    let text = format!(
        "clusters:\n- name: a\n  cluster:\n    server: https://a\n    certificate-authority: {}\n\
         contexts:\n- name: a\n  context:\n    cluster: a\n",
        ca.display()
    );
    let path = write(&dir, "kc", &text);

    let loaded = load(&[path]);

    let cluster = loaded.merged.clusters[0].cluster.as_ref().unwrap();
    assert_eq!(
        cluster.certificate_authority.as_deref(),
        ca.to_str(),
        "absolute path rewritten"
    );
}

#[test]
fn incompatible_kind_is_skipped_not_fatal() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));
    let other = write(
        &dir,
        "other",
        &config_yaml(&["b"], "x", None).replace("kind: Config", "kind: Other"),
    );

    let loaded = load(&[a, other.clone()]);

    assert_eq!(names(&loaded), ["a"]);
    assert!(matches!(
        loaded.diagnostics.as_slice(),
        [Diagnostic::Incompatible { path, .. }] if *path == other
    ));
    assert_eq!(loaded.sources[1].status, SourceStatus::Incompatible);
}

#[test]
fn same_file_listed_twice_loads_once() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));

    let loaded = load(&[a.clone(), a]);

    assert_eq!(loaded.sources.len(), 1);
    assert!(loaded.diagnostics.is_empty());
}

#[test]
fn no_paths_is_empty_and_tolerated() {
    let loaded = load(&[]);
    assert!(loaded.sources.is_empty());
    assert!(loaded.diagnostics.is_empty());
    assert!(loaded.merged.contexts.is_empty());
    assert!(!loaded.has_usable_source());
}

#[test]
fn require_usable_reports_not_found_when_everything_is_missing() {
    let dir = TempDir::new().unwrap();
    let paths = [dir.path().join("x"), dir.path().join("y")];

    let err = load_kubeconfig_from_paths_blocking(&paths, Strictness::RequireUsable).unwrap_err();

    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("x"), "{err}");
}

#[test]
fn require_usable_reports_not_found_for_an_empty_list() {
    let err = load_kubeconfig_from_paths_blocking(&[], Strictness::RequireUsable).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn require_usable_reports_validation_when_existing_files_are_unusable() {
    let dir = TempDir::new().unwrap();
    let blank = write(&dir, "blank", "");
    let corrupt = write(
        &dir,
        "corrupt",
        &format!("clusters: [\n  token: {SECRET}\n"),
    );
    let missing = dir.path().join("missing");

    let err =
        load_kubeconfig_from_paths_blocking(&[blank, corrupt, missing], Strictness::RequireUsable)
            .unwrap_err();

    assert_eq!(err.kind(), ErrorKind::Validation);
    assert!(!err.to_string().contains(SECRET));
}

#[test]
fn require_usable_succeeds_with_one_good_file() {
    let dir = TempDir::new().unwrap();
    let good = write(&dir, "good", &config_yaml(&["a"], "x", None));
    let missing = dir.path().join("missing");

    let loaded =
        load_kubeconfig_from_paths_blocking(&[missing, good], Strictness::RequireUsable).unwrap();

    assert_eq!(names(&loaded), ["a"]);
}

#[test]
fn debug_output_never_contains_credentials() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));
    let corrupt = write(&dir, "corrupt", &format!("users: [\n  token: {SECRET}\n"));

    let loaded = load(&[a, corrupt]);

    let text = format!("{loaded:?}");
    assert!(!text.contains(SECRET), "{text}");
    assert!(text.contains("\"a\""), "lists context names: {text}");
}

#[test]
fn is_blank_kubeconfig_distinguishes_empty_from_populated() {
    assert!(is_blank_kubeconfig(&Kubeconfig::default()));
    let populated = Kubeconfig::from_yaml(&config_yaml(&["a"], "x", None)).unwrap();
    assert!(!is_blank_kubeconfig(&populated));
    let only_current = Kubeconfig::from_yaml("current-context: a\n").unwrap();
    assert!(!is_blank_kubeconfig(&only_current));
}

#[tokio::test]
async fn local_load_uses_the_explicit_env_value() {
    let dir = TempDir::new().unwrap();
    let a = write(&dir, "a", &config_yaml(&["a"], "x", None));
    let b = write(&dir, "b", &config_yaml(&["b"], "x", None));
    let default = write(&dir, "default", &config_yaml(&["d"], "x", None));
    let env = std::env::join_paths([&a, &b]).unwrap();

    let loaded = load_local_kubeconfig(Some(env), Some(default), Strictness::Tolerant)
        .await
        .unwrap();

    assert_eq!(names(&loaded), ["a", "b"]);
}

#[tokio::test]
async fn local_load_falls_back_to_the_default_for_unset_or_empty_env() {
    let dir = TempDir::new().unwrap();
    let default = write(&dir, "default", &config_yaml(&["d"], "x", None));

    for env in [None, Some(OsString::new())] {
        let loaded = load_local_kubeconfig(env, Some(default.clone()), Strictness::Tolerant)
            .await
            .unwrap();
        assert_eq!(names(&loaded), ["d"]);
    }
}

#[tokio::test]
async fn local_load_with_no_inputs_is_not_found_when_strict() {
    let err = load_local_kubeconfig(None, None, Strictness::RequireUsable)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Any order of valid files with distinct context names yields the same set of names.
    #[test]
    fn permutations_yield_the_same_context_set(
        order in Just(vec![0usize, 1, 2, 3]).prop_shuffle()
    ) {
        let dir = TempDir::new().unwrap();
        let files: Vec<PathBuf> = [("f0", &["a", "b"][..]), ("f1", &["c"]), ("f2", &["d", "e"]), ("f3", &["f"])]
            .iter()
            .map(|(file, ctxs)| write(&dir, file, &config_yaml(ctxs, file, None)))
            .collect();
        let permuted: Vec<PathBuf> = order.iter().map(|&i| files[i].clone()).collect();

        let loaded = load(&permuted);

        let mut got = names(&loaded);
        got.sort();
        prop_assert_eq!(got, ["a", "b", "c", "d", "e", "f"]);
        prop_assert!(loaded.diagnostics.is_empty());
    }
}

/// Startup micro-benchmark for the PR (cold-start budget: docs/PERFORMANCE.md).
/// Run: `cargo test -p oxikube_kube --release --lib -- --ignored --nocapture bench_`.
#[test]
#[ignore = "timing report, not an assertion"]
fn bench_three_files_of_twenty_contexts() {
    let dir = TempDir::new().unwrap();
    let paths: Vec<PathBuf> = (0..3)
        .map(|f| {
            let names: Vec<String> = (0..20).map(|c| format!("ctx-{f}-{c}")).collect();
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            write(&dir, &format!("kc{f}"), &config_yaml(&refs, "bench", None))
        })
        .collect();
    let runs = 50;
    let mut times: Vec<_> = (0..runs)
        .map(|_| {
            let start = std::time::Instant::now();
            let loaded = load(&paths);
            assert_eq!(loaded.origins.len(), 60);
            start.elapsed()
        })
        .collect();
    times.sort();
    eprintln!(
        "load 3 files x 20 contexts: median {:?}, p95 {:?}, max {:?}",
        times[runs / 2],
        times[runs * 95 / 100],
        times[runs - 1]
    );
}
