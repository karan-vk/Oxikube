//! The RBAC kind fixtures (E17-S10): the [`RbacScenario`] applied to the cluster for one test and
//! deleted with it, what `kubectl auth can-i --list` answers for a subject, and a place to keep
//! both sides of a comparison that failed.
//!
//! | Helper | What it is for |
//! |---|---|
//! | [`AppliedScenario`] | the scenario's two `oxi-test-<rand>-*` namespaces, the restricted account, its Role and binding; deleted on drop |
//! | [`can_i_list`] | `kubectl auth can-i --list --as=<subject> -n <namespace>`, its rows without the header |
//! | [`save_diagnostic`] | a text the test compared, kept in the diagnostics directory CI uploads |
//!
//! Like the rest of this module it shells out to `kubectl --context <ctx>`: the testkit stays
//! free of `kube`. Nothing here creates or prints a token.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use oxikube_domain::rbac::Subject;

use super::{DIAGNOSTICS_DIR_ENV, IntegrationError, context_from, kubectl, random_namespace_name};
use crate::fixtures::rbac::RbacScenario;

/// The name a group is impersonated with: `kubectl --as-group` needs a user beside it.
const GROUP_MEMBER: &str = "oxi-member";

/// The [`RbacScenario`] on the cluster for one test: its namespaces are `oxi-test-<rand>-home`
/// and `oxi-test-<rand>-other`, deleted (with everything in them) when this is dropped, also when
/// the test panics. It creates nothing cluster-scoped.
#[derive(Debug)]
pub struct AppliedScenario {
    context: String,
    scenario: RbacScenario,
}

impl AppliedScenario {
    /// Applies a fresh scenario to the kind `context`.
    ///
    /// # Errors
    ///
    /// When `context` is not a kind context or `kubectl apply` fails.
    pub fn apply(context: &str) -> Result<Self, IntegrationError> {
        super::ensure_kind_context(context)?;
        let base = random_namespace_name();
        let scenario = RbacScenario::new(&format!("{base}-home"), &format!("{base}-other"));
        // Owned before the apply, so a half-applied scenario is deleted as well.
        let applied = Self {
            context: context.to_owned(),
            scenario,
        };
        apply_json(context, &applied.scenario.list())?;
        Ok(applied)
    }

    /// The scenario's names and objects.
    pub fn scenario(&self) -> &RbacScenario {
        &self.scenario
    }

    /// The kind context it was applied to.
    pub fn context(&self) -> &str {
        &self.context
    }
}

impl Drop for AppliedScenario {
    fn drop(&mut self) {
        // Best effort, never a panic in drop; the namespace controller finishes the job.
        let _ = kubectl(
            &self.context,
            &[
                "delete",
                "namespace",
                &self.scenario.home,
                &self.scenario.other,
                "--ignore-not-found",
                "--wait=false",
            ],
        );
    }
}

/// `kubectl apply -f -` of `object` (JSON) in `context`.
///
/// # Errors
///
/// When `kubectl` cannot be run or refuses the object.
fn apply_json(context: &str, object: &serde_json::Value) -> Result<(), IntegrationError> {
    let mut child = Command::new("kubectl")
        .args(["--context", context, "apply", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    child
        .stdin
        .take()
        .ok_or_else(|| IntegrationError::Kubectl("no stdin".to_owned()))?
        .write_all(object.to_string().as_bytes())
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    let out = child
        .wait_with_output()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(IntegrationError::Kubectl(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }
}

/// The impersonation flags that make `kubectl` ask as `subject`.
fn impersonation(subject: &Subject) -> Vec<String> {
    match subject {
        Subject::User(name) => vec![format!("--as={name}")],
        Subject::Group(group) => vec![
            format!("--as={GROUP_MEMBER}"),
            format!("--as-group={group}"),
        ],
        Subject::ServiceAccount { namespace, name } => {
            vec![format!("--as=system:serviceaccount:{namespace}:{name}")]
        }
    }
}

/// What `kubectl auth can-i --list` prints for `subject` in `namespace`, one row per entry, the
/// header dropped (read the rows back with
/// [`printed_rules`](crate::fixtures::rbac::printed_rules)).
///
/// # Errors
///
/// When `context` is not a kind context or `kubectl` fails.
pub fn can_i_list(
    context: &str,
    subject: &Subject,
    namespace: &str,
) -> Result<Vec<String>, IntegrationError> {
    super::ensure_kind_context(context)?;
    let out = Command::new("kubectl")
        .args([
            "--context",
            context,
            "auth",
            "can-i",
            "--list",
            "-n",
            namespace,
        ])
        .args(impersonation(subject))
        .output()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    if !out.status.success() {
        return Err(IntegrationError::Kubectl(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(str::to_owned)
        .collect())
}

/// Keeps `text` (one side of a comparison that is about to fail) as `<dir>/rbac/<file>` in the
/// diagnostics directory ([`DIAGNOSTICS_DIR_ENV`]), which CI uploads; on stderr when none is set.
/// Never fails. Callers pass rules and `kubectl` output only, never a token or a kubeconfig.
pub fn save_diagnostic(file: &str, text: &str) {
    if let Some(dir) = context_from(std::env::var(DIAGNOSTICS_DIR_ENV).ok()).map(PathBuf::from) {
        let dir = dir.join("rbac");
        let written =
            std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(dir.join(file), text));
        match written {
            Ok(()) => return,
            Err(e) => eprintln!("could not write {file} to {}: {e}", dir.display()),
        }
    }
    eprintln!("{file}:\n{text}");
}
