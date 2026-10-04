use std::fs;

use oxikube_domain::ids::ClusterId;
use tempfile::TempDir;

use super::*;
use crate::kubeconfig::incluster::{
    IN_CLUSTER_CONTEXT, SERVICE_ACCOUNT_CA_FILE, SERVICE_ACCOUNT_TOKEN_FILE,
};
use crate::kubeconfig::{Severity, in_cluster_cluster_id, in_cluster_context_name};

fn config_yaml(name: &str) -> String {
    format!(
        "apiVersion: v1\nkind: Config\nclusters:\n- name: {name}\n  cluster:\n    server: https://{name}.example\n\
         users:\n- name: {name}\n  user:\n    token: file-token\n\
         contexts:\n- name: {name}\n  context:\n    cluster: {name}\n    user: {name}\n"
    )
}

fn write(dir: &TempDir, file: &str, text: &str) -> PathBuf {
    let path = dir.path().join(file);
    fs::write(&path, text).unwrap();
    path
}

fn kubeconfig_var(paths: &[&PathBuf]) -> Option<OsString> {
    Some(std::env::join_paths(paths).unwrap())
}

/// An environment that looks like a pod.
fn pod_env() -> Env {
    Env {
        kubernetes_service_host: Some("10.0.0.1".into()),
        kubernetes_service_port: Some("443".into()),
        service_account_mounted: true,
        service_account_namespace: Some("team-a".into()),
        ..Env::default()
    }
}

fn load(explicit: &[PathBuf], env: &Env) -> LoadedKubeconfig {
    load_kubeconfig_for_env_blocking(explicit, env, Strictness::Tolerant).unwrap()
}

fn names(loaded: &LoadedKubeconfig) -> Vec<String> {
    loaded.context_names().map(|c| c.to_string()).collect()
}

fn in_cluster_ran(loaded: &LoadedKubeconfig) -> bool {
    loaded.diagnostics.contains(&Diagnostic::InClusterUsed)
}

// --- source selection (pure) -------------------------------------------------------------

#[test]
fn explicit_beats_kubeconfig_env_and_default() {
    let env = Env {
        kubeconfig: kubeconfig_var(&[&PathBuf::from("/env/a")]),
        home: Some("/home/u".into()),
        ..Env::default()
    };
    let selection = select_sources(&[PathBuf::from("/explicit")], &env);
    assert_eq!(selection.tier, Some(SourceTier::Explicit));
    assert_eq!(selection.paths, [PathBuf::from("/explicit")]);
}

#[test]
fn kubeconfig_env_beats_default() {
    let a = PathBuf::from("/env/a");
    let b = PathBuf::from("/env/b");
    let env = Env {
        kubeconfig: kubeconfig_var(&[&a, &b]),
        home: Some("/home/u".into()),
        ..Env::default()
    };
    let selection = select_sources(&[], &env);
    assert_eq!(selection.tier, Some(SourceTier::KubeconfigEnv));
    assert_eq!(selection.paths, [a, b]);
}

#[test]
fn default_path_when_nothing_else_is_given() {
    let env = Env {
        home: Some("/home/u".into()),
        ..Env::default()
    };
    let selection = select_sources(&[], &env);
    assert_eq!(selection.tier, Some(SourceTier::DefaultPath));
    assert_eq!(
        selection.paths,
        [PathBuf::from("/home/u").join(".kube").join("config")]
    );
}

#[test]
fn empty_inputs_fall_through_each_tier() {
    let env = Env {
        kubeconfig: Some(OsString::from("")),
        home: Some("/home/u".into()),
        ..Env::default()
    };
    // Empty explicit paths and an empty KUBECONFIG count as unset.
    let selection = select_sources(&[PathBuf::new()], &env);
    assert_eq!(selection.tier, Some(SourceTier::DefaultPath));
}

#[test]
fn separators_only_kubeconfig_selects_the_env_tier_with_no_paths() {
    // kubectl only uses the home file when KUBECONFIG is empty; ":" is not empty.
    let sep = if cfg!(windows) { ";" } else { ":" };
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".kube")).unwrap();
    fs::write(
        home.path().join(".kube").join("config"),
        config_yaml("default"),
    )
    .unwrap();
    let env = Env {
        kubeconfig: Some(OsString::from(format!("{sep}{sep}"))),
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };

    let selection = select_sources(&[], &env);
    assert_eq!(selection.tier, Some(SourceTier::KubeconfigEnv));
    assert!(selection.paths.is_empty());

    let loaded = load(&[], &env);
    assert!(names(&loaded).is_empty(), "default file must not be loaded");
    assert!(matches!(
        loaded.diagnostics[0],
        Diagnostic::SourceSelected {
            tier: SourceTier::KubeconfigEnv,
            paths: 0
        }
    ));
}

#[test]
fn nothing_to_select() {
    let selection = select_sources(&[], &Env::default());
    assert_eq!(
        selection,
        Selection {
            tier: None,
            paths: vec![]
        }
    );
}

#[test]
fn kubeconfig_env_uses_the_envs_platform_rules() {
    let env = Env {
        platform: Platform::Windows,
        kubeconfig: Some(OsString::from("C:\\a\\config;D:\\b\\config")),
        ..Env::default()
    };
    assert_eq!(
        select_sources(&[], &env).paths,
        [
            PathBuf::from("C:\\a\\config"),
            PathBuf::from("D:\\b\\config")
        ]
    );
    let env = Env {
        platform: Platform::Unix,
        kubeconfig: Some(OsString::from("/a/config:/b/config")),
        ..Env::default()
    };
    assert_eq!(
        select_sources(&[], &env).paths,
        [PathBuf::from("/a/config"), PathBuf::from("/b/config")]
    );
}

// --- precedence through the loader ---------------------------------------------------------

#[test]
fn explicit_source_wins_over_kubeconfig_env_and_default() {
    let dir = TempDir::new().unwrap();
    let explicit = write(&dir, "explicit", &config_yaml("explicit"));
    let from_env = write(&dir, "env", &config_yaml("from-env"));
    let default_home = TempDir::new().unwrap();
    fs::create_dir(default_home.path().join(".kube")).unwrap();
    fs::write(
        default_home.path().join(".kube").join("config"),
        config_yaml("default"),
    )
    .unwrap();
    let env = Env {
        kubeconfig: kubeconfig_var(&[&from_env]),
        home: Some(default_home.path().to_path_buf()),
        ..pod_env()
    };

    let loaded = load(&[explicit], &env);

    assert_eq!(names(&loaded), ["explicit"]);
    assert!(matches!(
        loaded.diagnostics[0],
        Diagnostic::SourceSelected {
            tier: SourceTier::Explicit,
            paths: 1
        }
    ));
    assert!(!in_cluster_ran(&loaded));
}

#[test]
fn kubeconfig_env_wins_over_default_and_does_not_fall_through() {
    let dir = TempDir::new().unwrap();
    let from_env = write(&dir, "env", &config_yaml("from-env"));
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".kube")).unwrap();
    fs::write(
        home.path().join(".kube").join("config"),
        config_yaml("default"),
    )
    .unwrap();
    let env = Env {
        kubeconfig: kubeconfig_var(&[&from_env]),
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };
    assert_eq!(names(&load(&[], &env)), ["from-env"]);

    // A KUBECONFIG whose files are all missing does not fall back to the default path.
    let env = Env {
        kubeconfig: kubeconfig_var(&[&dir.path().join("missing")]),
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };
    assert!(names(&load(&[], &env)).is_empty());
}

#[test]
fn default_path_is_used_when_nothing_else_is_set() {
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".kube")).unwrap();
    fs::write(
        home.path().join(".kube").join("config"),
        config_yaml("default"),
    )
    .unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };

    let loaded = load(&[], &env);

    assert_eq!(names(&loaded), ["default"]);
    assert!(matches!(
        loaded.diagnostics[0],
        Diagnostic::SourceSelected {
            tier: SourceTier::DefaultPath,
            ..
        }
    ));
}

// --- in-cluster fallback ---------------------------------------------------------------------

#[test]
fn in_cluster_is_used_only_when_nothing_else_works() {
    let home = TempDir::new().unwrap(); // no ~/.kube/config
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    assert_eq!(names(&loaded), [IN_CLUSTER_CONTEXT]);
    assert_eq!(
        loaded.merged.current_context.as_deref(),
        Some(IN_CLUSTER_CONTEXT)
    );
    assert!(in_cluster_ran(&loaded));
    assert!(loaded.has_usable_source());
    let source = loaded.sources.last().unwrap();
    assert!(source.is_in_cluster());
    assert_eq!(source.status, SourceStatus::Loaded);
    // The missing default file stays listed.
    assert_eq!(loaded.sources[0].status, SourceStatus::Missing);
}

#[test]
fn in_cluster_context_is_invisible_when_a_kubeconfig_has_contexts() {
    let dir = TempDir::new().unwrap();
    let good = write(&dir, "good", &config_yaml("prod"));
    let env = Env {
        kubeconfig: kubeconfig_var(&[&good]),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    assert_eq!(names(&loaded), ["prod"]);
    assert!(loaded.diagnostics.iter().all(|d| !matches!(
        d,
        Diagnostic::InClusterUsed | Diagnostic::InClusterSkipped { .. }
    )));
    assert!(loaded.sources.iter().all(|s| !s.is_in_cluster()));
}

#[test]
fn corrupt_kubeconfig_never_falls_back_to_in_cluster() {
    let dir = TempDir::new().unwrap();
    let corrupt = write(&dir, "corrupt", "clusters: [\n  : : :\n");
    let env = Env {
        kubeconfig: kubeconfig_var(&[&corrupt]),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    assert!(names(&loaded).is_empty());
    assert!(!in_cluster_ran(&loaded));
    assert!(loaded.sources.iter().all(|s| !s.is_in_cluster()));
    assert!(
        loaded
            .diagnostics
            .contains(&Diagnostic::Unparsable { path: corrupt })
    );
    let skipped = Diagnostic::InClusterSkipped {
        reason: InClusterSkip::BrokenKubeconfig,
    };
    assert!(loaded.diagnostics.contains(&skipped));
    assert_eq!(skipped.severity(), Severity::Warning);
}

#[test]
fn corrupt_file_next_to_a_blank_one_still_blocks_the_fallback() {
    let dir = TempDir::new().unwrap();
    let blank = write(&dir, "blank", "");
    let corrupt = write(&dir, "corrupt", "users: [\n");
    let env = Env {
        kubeconfig: kubeconfig_var(&[&blank, &corrupt]),
        ..pod_env()
    };
    assert!(!in_cluster_ran(&load(&[], &env)));
}

#[test]
fn blank_or_missing_kubeconfig_allows_the_fallback() {
    let dir = TempDir::new().unwrap();
    let blank = write(&dir, "blank", "  \n");
    let missing = dir.path().join("missing");
    let env = Env {
        kubeconfig: kubeconfig_var(&[&blank, &missing]),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    assert!(in_cluster_ran(&loaded));
    assert_eq!(names(&loaded), [IN_CLUSTER_CONTEXT]);
}

#[test]
fn not_in_a_pod_means_no_fallback() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };

    let loaded = load(&[], &env);

    assert!(names(&loaded).is_empty());
    let skipped = Diagnostic::InClusterSkipped {
        reason: InClusterSkip::NotInCluster,
    };
    assert!(loaded.diagnostics.contains(&skipped));
    assert_eq!(skipped.severity(), Severity::Info);
}

#[test]
fn diagnostics_record_the_winning_source_and_the_fallback() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    let text: Vec<String> = loaded.diagnostics.iter().map(|d| d.to_string()).collect();
    assert!(text[0].contains("default kubeconfig path"), "{text:?}");
    assert!(text.last().unwrap().contains("in-cluster"), "{text:?}");
}

#[test]
fn no_paths_at_all_is_reported() {
    let loaded = load(&[], &Env::default());
    assert_eq!(loaded.diagnostics[0], Diagnostic::NoKubeconfigSource);
}

#[test]
fn detection_table() {
    let mut cases: Vec<(&str, Env, bool)> = vec![
        ("pod", pod_env(), true),
        ("default env", Env::default(), false),
        (
            "not mounted",
            Env {
                service_account_mounted: false,
                ..pod_env()
            },
            false,
        ),
        (
            "no host",
            Env {
                kubernetes_service_host: None,
                ..pod_env()
            },
            false,
        ),
        (
            "no port",
            Env {
                kubernetes_service_port: None,
                ..pod_env()
            },
            false,
        ),
        (
            "bad port",
            Env {
                kubernetes_service_port: Some("https".into()),
                ..pod_env()
            },
            false,
        ),
        (
            "empty host",
            Env {
                kubernetes_service_host: Some(String::new()),
                ..pod_env()
            },
            false,
        ),
    ];
    cases.push((
        "ipv6",
        Env {
            kubernetes_service_host: Some("fd00::1".into()),
            kubernetes_service_port: Some("6443".into()),
            ..pod_env()
        },
        true,
    ));
    for (label, env, expected) in cases {
        assert_eq!(env.in_cluster_detected(), expected, "{label}");
    }
}

// --- in-cluster context shape and secrecy ----------------------------------------------------

#[test]
fn in_cluster_context_mirrors_kube_incluster_and_holds_no_token() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        kubernetes_service_host: Some("10.1.2.3".into()),
        kubernetes_service_port: Some("6443".into()),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    let cluster = loaded.merged.clusters[0].cluster.as_ref().unwrap();
    assert_eq!(cluster.server.as_deref(), Some("https://10.1.2.3:6443"));
    assert_eq!(
        cluster.certificate_authority.as_deref(),
        Some(SERVICE_ACCOUNT_CA_FILE)
    );
    let user = loaded.merged.auth_infos[0].auth_info.as_ref().unwrap();
    assert_eq!(user.token_file.as_deref(), Some(SERVICE_ACCOUNT_TOKEN_FILE));
    assert!(user.token.is_none() && user.password.is_none());
    let context = loaded.merged.contexts[0].context.as_ref().unwrap();
    assert_eq!(context.namespace.as_deref(), Some("team-a"));
    assert_eq!(context.cluster, IN_CLUSTER_CONTEXT);
    assert_eq!(context.user.as_deref(), Some(IN_CLUSTER_CONTEXT));

    // Only the token file's path appears anywhere an operator could see; there is no token.
    let text = format!(
        "{loaded:?} {:?}",
        loaded
            .diagnostics
            .iter()
            .map(|d| d.to_string())
            .collect::<Vec<_>>()
    );
    assert!(!text.to_lowercase().contains("bearer"), "{text}");
    assert!(!text.contains(SERVICE_ACCOUNT_TOKEN_FILE), "{text}");
    assert!(
        !format!("{env:?}").contains("token"),
        "Env has no token field"
    );
}

#[test]
fn in_cluster_cluster_id_uses_the_fixed_label_and_is_stable() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..pod_env()
    };

    let loaded = load(&[], &env);

    let id = loaded.cluster_id(&in_cluster_context_name()).unwrap();
    assert_eq!(id, in_cluster_cluster_id());
    assert_eq!(id, ClusterId::new("in-cluster", &in_cluster_context_name()));
    assert_eq!(
        loaded.origin(&in_cluster_context_name()),
        Some(std::path::Path::new(
            super::super::incluster::IN_CLUSTER_SOURCE_PATH
        ))
    );
}

// --- strictness ----------------------------------------------------------------------------

#[test]
fn strict_load_with_no_sources_and_not_in_a_cluster_is_not_found() {
    let err = load_kubeconfig_for_env_blocking(&[], &Env::default(), Strictness::RequireUsable)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
    assert!(err.message().contains("not running in a cluster"), "{err}");
}

#[test]
fn strict_load_with_a_missing_default_file_is_not_found() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..Env::default()
    };
    let err = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

#[test]
fn strict_load_with_a_corrupt_file_in_a_pod_is_validation_not_in_cluster() {
    let dir = TempDir::new().unwrap();
    let corrupt = write(&dir, "corrupt", "users: [\n");
    let env = Env {
        kubeconfig: kubeconfig_var(&[&corrupt]),
        ..pod_env()
    };
    let err = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Validation);
}

#[test]
fn strict_load_succeeds_through_the_fallback() {
    let home = TempDir::new().unwrap();
    let env = Env {
        home: Some(home.path().to_path_buf()),
        ..pod_env()
    };
    let loaded = load_kubeconfig_for_env_blocking(&[], &env, Strictness::RequireUsable).unwrap();
    assert_eq!(names(&loaded), [IN_CLUSTER_CONTEXT]);
}

#[tokio::test]
async fn async_entry_point_matches_the_blocking_one() {
    let dir = TempDir::new().unwrap();
    let good = write(&dir, "good", &config_yaml("prod"));
    let env = Env {
        kubeconfig: kubeconfig_var(&[&good]),
        ..Env::default()
    };
    let loaded = load_kubeconfig_for_env(vec![], env, Strictness::Tolerant)
        .await
        .unwrap();
    assert_eq!(names(&loaded), ["prod"]);
}

#[test]
fn existing_file_only_api_is_unchanged_and_never_falls_back() {
    // The S01 entry points stay file-only: no in-cluster even with a pod-like environment.
    let loaded =
        crate::kubeconfig::load_kubeconfig_from_paths_blocking(&[], Strictness::Tolerant).unwrap();
    assert!(loaded.merged.contexts.is_empty());
    assert!(loaded.diagnostics.is_empty());
}

// --- the exported fallback step ----------------------------------------------------------------

fn files_only(paths: &[PathBuf]) -> LoadedKubeconfig {
    crate::kubeconfig::load_kubeconfig_from_paths_blocking(paths, Strictness::Tolerant).unwrap()
}

#[test]
fn fallback_step_adds_the_context_after_a_caller_loaded_its_own_files() {
    let dir = TempDir::new().unwrap();
    let blank = write(&dir, "blank", "");
    let mut loaded = files_only(&[blank, dir.path().join("missing")]);

    apply_in_cluster_fallback(&mut loaded, &pod_env()).unwrap();

    assert_eq!(names(&loaded), [IN_CLUSTER_CONTEXT]);
    assert_eq!(loaded.diagnostics.last(), Some(&Diagnostic::InClusterUsed));
    let source = loaded.sources.last().unwrap();
    assert!(source.is_in_cluster());
    assert_eq!(
        source.path,
        PathBuf::from(crate::kubeconfig::IN_CLUSTER_SOURCE_PATH)
    );
    assert_eq!(
        loaded.cluster_id(&in_cluster_context_name()),
        Some(in_cluster_cluster_id())
    );
    // Calling it again changes nothing.
    let before = (loaded.sources.len(), loaded.diagnostics.len());
    apply_in_cluster_fallback(&mut loaded, &pod_env()).unwrap();
    assert_eq!((loaded.sources.len(), loaded.diagnostics.len()), before);
}

#[test]
fn fallback_step_refuses_over_a_broken_file_and_records_why() {
    let dir = TempDir::new().unwrap();
    let corrupt = write(&dir, "corrupt", "users: [\n");
    let mut loaded = files_only(&[corrupt]);

    apply_in_cluster_fallback(&mut loaded, &pod_env()).unwrap();

    assert!(names(&loaded).is_empty());
    assert_eq!(
        loaded.diagnostics.last(),
        Some(&Diagnostic::InClusterSkipped {
            reason: InClusterSkip::BrokenKubeconfig
        })
    );
}

#[test]
fn fallback_step_outside_a_pod_records_not_in_cluster() {
    let mut loaded = files_only(&[]);

    apply_in_cluster_fallback(&mut loaded, &Env::default()).unwrap();

    assert!(names(&loaded).is_empty());
    assert_eq!(
        loaded.diagnostics.last(),
        Some(&Diagnostic::InClusterSkipped {
            reason: InClusterSkip::NotInCluster
        })
    );
}

#[test]
fn fallback_step_is_a_no_op_when_a_context_exists() {
    let dir = TempDir::new().unwrap();
    let good = write(&dir, "good", &config_yaml("prod"));
    let mut loaded = files_only(&[good]);

    apply_in_cluster_fallback(&mut loaded, &pod_env()).unwrap();

    assert_eq!(names(&loaded), ["prod"]);
    assert!(loaded.diagnostics.is_empty());
}
