//! The build command line and what a failed build means.
//!
//! Safety: the line never carries `--enable-exec`, `--enable-alpha-plugins` or `--enable-helm`
//! (an untrusted overlay could otherwise run code), and passes the default
//! `--load-restrictor=LoadRestrictionsRootOnly` explicitly so it can not be loosened by
//! accident. A build that fails because it needs one of those features says so, with the
//! terminal command to run instead.

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitStatus;

use oxikube_domain::kustomize::{KustomizeError, KustomizeToolKind};

/// Lines of stderr kept in an error (the end, where kustomize puts the cause).
const STDERR_LINES: usize = 40;

/// The arguments after the program name: `build` for the standalone tool, `kustomize` for
/// `kubectl`, then the flags, then `--` and the directory (so a directory called `-x` is a
/// path).
pub(crate) fn args(kind: KustomizeToolKind, dir: &Path) -> Vec<OsString> {
    let first = match kind {
        KustomizeToolKind::Standalone => "build",
        KustomizeToolKind::Kubectl => "kustomize",
    };
    vec![
        first.into(),
        "--load-restrictor=LoadRestrictionsRootOnly".into(),
        "--".into(),
        dir.as_os_str().to_owned(),
    ]
}

/// Redacted, trimmed stderr: the last [`STDERR_LINES`] lines, with secret-looking text masked.
pub(crate) fn clean_stderr(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.trim().lines().collect();
    let tail = &lines[lines.len().saturating_sub(STDERR_LINES)..];
    oxikube_domain::redact::redact(&tail.join("\n")).into_owned()
}

/// The error for a build that exited unsuccessfully.
pub(crate) fn failure(kind: KustomizeToolKind, status: ExitStatus, stderr: &str) -> KustomizeError {
    let stderr = clean_stderr(stderr);
    let lower = stderr.to_lowercase();
    let disabled = if lower.contains("enable-helm") || lower.contains("helmcharts") {
        Some(("the Helm chart inflator", "enable-helm"))
    } else if lower.contains("enable-alpha-plugins")
        || lower.contains("enable-exec")
        || lower.contains("unable to find plugin root")
    {
        Some(("exec or alpha plugins", "enable-alpha-plugins"))
    } else {
        None
    };
    match disabled {
        Some((feature, flag)) => KustomizeError::FeatureDisabled {
            feature,
            flag,
            command: kind.command(),
            stderr,
        },
        None => KustomizeError::Failed {
            exit_code: status.code(),
            stderr: if stderr.is_empty() {
                format!("{} exited with {status}", kind.command())
            } else {
                stderr
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_has_no_dangerous_flags() {
        for kind in [KustomizeToolKind::Standalone, KustomizeToolKind::Kubectl] {
            let line = args(kind, Path::new("/o/prod"));
            let text: Vec<_> = line
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            assert!(!text.iter().any(|a| a.contains("enable-")), "{text:?}");
            assert_eq!(text[text.len() - 2..], ["--", "/o/prod"]);
        }
        assert_eq!(
            args(KustomizeToolKind::Standalone, Path::new("d"))[0],
            "build"
        );
        assert_eq!(
            args(KustomizeToolKind::Kubectl, Path::new("d"))[0],
            "kustomize"
        );
    }

    #[test]
    fn stderr_keeps_its_tail() {
        let long: String = (0..100).map(|n| format!("line {n}\n")).collect();
        let cleaned = clean_stderr(&long);
        assert_eq!(cleaned.lines().count(), STDERR_LINES);
        assert!(cleaned.ends_with("line 99"));
    }
}
