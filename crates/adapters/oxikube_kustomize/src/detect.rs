//! Finding a tool: the standalone `kustomize` first (`kustomize version`), then
//! `kubectl kustomize` (`kubectl kustomize --help` proves the subcommand exists,
//! `kubectl version --client` names the bundled kustomize version).

use std::ffi::OsString;
use std::path::Path;

use oxikube_domain::kustomize::{KustomizeTool, KustomizeToolKind};

use crate::config::KustomizeConfig;
use crate::path::{Folders, is_executable};
use crate::run::run;

/// The version shown when a tool works but will not say which kustomize it is.
const UNKNOWN_VERSION: &str = "unknown";

/// Looks for a tool: the configured binary alone when `config.path` is set, else the standalone
/// binary, else `kubectl`.
pub(crate) async fn detect(config: &KustomizeConfig, folders: &Folders) -> Option<KustomizeTool> {
    if let Some(path) = &config.path {
        if !is_executable(path) {
            return None;
        }
        return match kind_of(path) {
            KustomizeToolKind::Kubectl => probe_kubectl(path, config, folders).await,
            KustomizeToolKind::Standalone => probe_standalone(path, config, folders).await,
        };
    }
    if let Some(path) = folders.find("kustomize")
        && let Some(tool) = probe_standalone(&path, config, folders).await
    {
        return Some(tool);
    }
    let path = folders.find("kubectl")?;
    probe_kubectl(&path, config, folders).await
}

/// A configured file named `kubectl` selects the bundled kustomize; any other name is a
/// standalone kustomize.
fn kind_of(path: &Path) -> KustomizeToolKind {
    let stem = path.file_stem().and_then(|stem| stem.to_str());
    if stem == Some("kubectl") {
        KustomizeToolKind::Kubectl
    } else {
        KustomizeToolKind::Standalone
    }
}

fn args(words: &[&str]) -> Vec<OsString> {
    words.iter().map(OsString::from).collect()
}

async fn probe_standalone(
    path: &Path,
    config: &KustomizeConfig,
    folders: &Folders,
) -> Option<KustomizeTool> {
    let done = run(
        path,
        &args(&["version"]),
        folders.as_path_var(),
        config.probe_timeout,
    )
    .await
    .ok()?;
    done.status.success().then(|| KustomizeTool {
        kind: KustomizeToolKind::Standalone,
        version: parse_standalone_version(&done.stdout)
            .unwrap_or_else(|| UNKNOWN_VERSION.to_owned()),
        path: path.to_path_buf(),
    })
}

async fn probe_kubectl(
    path: &Path,
    config: &KustomizeConfig,
    folders: &Folders,
) -> Option<KustomizeTool> {
    let var = folders.as_path_var();
    let help = run(
        path,
        &args(&["kustomize", "--help"]),
        var,
        config.probe_timeout,
    )
    .await
    .ok()?;
    if !help.status.success() {
        return None;
    }
    let version = run(
        path,
        &args(&["version", "--client"]),
        var,
        config.probe_timeout,
    )
    .await
    .ok()
    .filter(|done| done.status.success())
    .and_then(|done| parse_kubectl_version(&done.stdout))
    .unwrap_or_else(|| UNKNOWN_VERSION.to_owned());
    Some(KustomizeTool {
        kind: KustomizeToolKind::Kubectl,
        version,
        path: path.to_path_buf(),
    })
}

/// The kustomize version in `kustomize version` output: `v5.4.2` today,
/// `{Version:kustomize/v4.5.7 GitCommit:... }` before kustomize 5.
pub(crate) fn parse_standalone_version(output: &str) -> Option<String> {
    find_version(output)
}

/// The kustomize version in `kubectl version --client` output (its `Kustomize Version:` line).
pub(crate) fn parse_kubectl_version(output: &str) -> Option<String> {
    let line = output
        .lines()
        .find(|line| line.trim_start().starts_with("Kustomize Version"))?;
    find_version(line)
}

/// The first `vMAJOR.MINOR.PATCH` (with any `-pre`/`+build` suffix) in `text`.
fn find_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    (0..bytes.len())
        .filter(|&at| bytes[at] == b'v' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        .find_map(|at| {
            let word: String = text[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
                .collect();
            let numbers = word[1..].split(['-', '+']).next()?;
            let parts: Vec<_> = numbers.split('.').collect();
            let all_digits = parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
            (parts.len() == 3 && all_digits).then_some(word)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_versions_new_and_old() {
        assert_eq!(
            parse_standalone_version("v5.4.2\n").as_deref(),
            Some("v5.4.2")
        );
        let old = "{Version:kustomize/v4.5.7 GitCommit:56d82a8 BuildDate:2022 GoOs:linux}";
        assert_eq!(parse_standalone_version(old).as_deref(), Some("v4.5.7"));
        assert_eq!(parse_standalone_version("no version here"), None);
        assert_eq!(
            parse_standalone_version("v5.4"),
            None,
            "needs three numbers"
        );
    }

    #[test]
    fn kubectl_reports_its_bundled_kustomize() {
        let out =
            "Client Version: v1.30.0\nKustomize Version: v5.0.4-0.20230601165947-6ce0bf390ce3\n";
        assert_eq!(
            parse_kubectl_version(out).as_deref(),
            Some("v5.0.4-0.20230601165947-6ce0bf390ce3")
        );
        assert_eq!(
            parse_kubectl_version("Client Version: v1.30.0\n"),
            None,
            "the kubectl version is not the kustomize version"
        );
    }

    #[test]
    fn a_kubectl_file_name_selects_the_bundled_kustomize() {
        assert_eq!(kind_of(Path::new("/x/kubectl")), KustomizeToolKind::Kubectl);
        assert_eq!(
            kind_of(Path::new("/x/kubectl.exe")),
            KustomizeToolKind::Kubectl
        );
        assert_eq!(
            kind_of(Path::new("/x/kustomize5")),
            KustomizeToolKind::Standalone
        );
    }
}
