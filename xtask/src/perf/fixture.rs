//! The kubeconfig fixture of the `startup` scenario: 3 kubeconfig files with 20 contexts in all,
//! the setup the cold-start budget is defined on (docs/PERFORMANCE.md "Budgets").
//!
//! Every scenario process gets `KUBECONFIG` pointing at the three files, so start-up runs against
//! the reference setup. Nothing may read them before the first frame (the catalog loads them on
//! `spawn_kube` afterwards, E06); the servers are unroutable loopback addresses so that a stray
//! connection attempt fails at once instead of reaching anything.

use anyhow::{Context, Result};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::Path;

/// Contexts per file: 7 + 7 + 6 = 20.
pub const CONTEXTS_PER_FILE: [usize; 3] = [7, 7, 6];

/// Writes the three kubeconfigs into `dir` and returns the `KUBECONFIG` value listing them.
pub fn write_kubeconfigs(dir: &Path) -> Result<OsString> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let mut paths = Vec::new();
    let mut next = 0;
    for (file, contexts) in CONTEXTS_PER_FILE.iter().enumerate() {
        let path = dir.join(format!("kubeconfig-{}.yaml", file + 1));
        std::fs::write(&path, kubeconfig(next, *contexts))
            .with_context(|| format!("writing {}", path.display()))?;
        paths.push(path);
        next += contexts;
    }
    std::env::join_paths(paths).context("joining the kubeconfig paths")
}

/// A kubeconfig with `count` contexts numbered from `first`, each with its own cluster and user.
fn kubeconfig(first: usize, count: usize) -> String {
    let mut clusters = String::new();
    let mut users = String::new();
    let mut contexts = String::new();
    for i in first..first + count {
        let _ = write!(
            clusters,
            "- name: perf-cluster-{i}\n  cluster:\n    server: https://127.0.0.1:{}\n    insecure-skip-tls-verify: true\n",
            // Port 1 upwards on loopback: nothing listens, a connection is refused at once.
            i + 1
        );
        let _ = write!(
            users,
            "- name: perf-user-{i}\n  user:\n    token: perf-fixture-not-a-secret\n"
        );
        let _ = write!(
            contexts,
            "- name: perf-{i}\n  context:\n    cluster: perf-cluster-{i}\n    user: perf-user-{i}\n    namespace: default\n"
        );
    }
    format!(
        "apiVersion: v1\nkind: Config\ncurrent-context: perf-{first}\nclusters:\n{clusters}users:\n{users}contexts:\n{contexts}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_files_with_twenty_contexts() {
        let dir = std::env::temp_dir().join(format!("xtask-kubeconfig-{}", std::process::id()));
        let value = write_kubeconfigs(&dir).unwrap();
        let files: Vec<_> = std::env::split_paths(&value).collect();
        assert_eq!(files.len(), 3);
        let contexts: usize = files
            .iter()
            .map(|f| {
                std::fs::read_to_string(f)
                    .unwrap()
                    .lines()
                    .filter(|l| {
                        l.starts_with("- name: perf-")
                            && !l.contains("cluster")
                            && !l.contains("user")
                    })
                    .count()
            })
            .sum();
        assert_eq!(contexts, 20);
        assert!(
            std::fs::read_to_string(&files[0])
                .unwrap()
                .contains("current-context: perf-0")
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
