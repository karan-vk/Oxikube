//! Helpers for kind-backed integration tests (feature `integration`).
//!
//! The cluster comes from `cargo xtask kind-up`; tests learn its kubectl context from
//! [`CONTEXT_ENV`] and skip cleanly when it is unset. Each test creates its own
//! `oxi-test-<rand>` namespace via [`TestNamespace`], which is deleted on drop.
//!
//! This shells out to `kubectl --context <ctx>` on purpose: the testkit stays free of
//! `kube`, and every call is pinned to the named context.

use std::fmt;
use std::process::Command;

/// Environment variable naming the kubectl context of the kind cluster under test.
pub const CONTEXT_ENV: &str = "OXIKUBE_TEST_CONTEXT";

/// Prefix of every namespace created by tests.
pub const NAMESPACE_PREFIX: &str = "oxi-test-";

#[derive(Debug)]
pub enum IntegrationError {
    /// The context is not a kind context; refusing to touch it.
    NotKindContext(String),
    /// `kubectl` could not be run or exited non-zero.
    Kubectl(String),
}

impl fmt::Display for IntegrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotKindContext(c) => {
                write!(
                    f,
                    "refusing to use non-kind context `{c}` (expected `kind-*`)"
                )
            }
            Self::Kubectl(m) => write!(f, "kubectl failed: {m}"),
        }
    }
}

impl std::error::Error for IntegrationError {}

/// Interpret the raw value of [`CONTEXT_ENV`]: unset or blank means "skip".
pub fn context_from(raw: Option<String>) -> Option<String> {
    raw.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// The kind context to test against, or `None` (after printing a skip notice) when
/// [`CONTEXT_ENV`] is unset. Call at the top of every integration test and return early.
pub fn test_context() -> Option<String> {
    let ctx = context_from(std::env::var(CONTEXT_ENV).ok());
    if ctx.is_none() {
        eprintln!(
            "skipping integration test: {CONTEXT_ENV} is not set (run `cargo xtask kind-up`)"
        );
    }
    ctx
}

/// Only kind contexts may be used by tests; they create and delete namespaces.
pub fn ensure_kind_context(context: &str) -> Result<(), IntegrationError> {
    if context.starts_with("kind-") {
        Ok(())
    } else {
        Err(IntegrationError::NotKindContext(context.to_owned()))
    }
}

/// A fresh, random, DNS-label-safe test namespace name: `oxi-test-<8 hex>`.
pub fn random_namespace_name() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("{NAMESPACE_PREFIX}{}", &id[..8])
}

fn kubectl(context: &str, args: &[&str]) -> Result<(), IntegrationError> {
    let out = Command::new("kubectl")
        .arg("--context")
        .arg(context)
        .args(args)
        .output()
        .map_err(|e| IntegrationError::Kubectl(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(IntegrationError::Kubectl(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }
}

/// A namespace created for one test and deleted when dropped (also on panic).
#[derive(Debug)]
pub struct TestNamespace {
    context: String,
    name: String,
}

impl TestNamespace {
    /// Create `oxi-test-<rand>` in the given kind context.
    pub fn create(context: &str) -> Result<Self, IntegrationError> {
        ensure_kind_context(context)?;
        let name = random_namespace_name();
        kubectl(context, &["create", "namespace", &name])?;
        Ok(Self {
            context: context.to_owned(),
            name,
        })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn context(&self) -> &str {
        &self.context
    }

    /// The namespace's events, oldest first, as `kubectl get events -o wide` prints them, for a
    /// test that times out to show what the cluster did. Never fails: when kubectl cannot be run
    /// or exits non-zero, the text says so instead. Events carry reasons and messages written by
    /// controllers, not object payloads.
    pub fn events(&self) -> String {
        let out = Command::new("kubectl")
            .args(["--context", &self.context, "-n", &self.name])
            .args(["get", "events", "-o", "wide"])
            .arg("--sort-by=.metadata.creationTimestamp")
            .output();
        match out {
            Ok(out) if out.status.success() => {
                let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                if text.is_empty() {
                    format!("(no events in {})", self.name)
                } else {
                    text
                }
            }
            Ok(out) => format!(
                "(kubectl get events failed: {})",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(e) => format!("(kubectl could not be run: {e})"),
        }
    }
}

impl Drop for TestNamespace {
    fn drop(&mut self) {
        // Best effort: never panic in drop. `--wait=false` keeps test teardown fast; the
        // namespace controller finishes the job.
        let _ = kubectl(
            &self.context,
            &[
                "delete",
                "namespace",
                &self.name,
                "--ignore-not-found",
                "--wait=false",
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_blank_context_skips() {
        assert_eq!(context_from(None), None);
        assert_eq!(context_from(Some(String::new())), None);
        assert_eq!(context_from(Some("  \n".into())), None);
    }

    #[test]
    fn context_is_trimmed() {
        assert_eq!(
            context_from(Some(" kind-oxikube\n".into())).as_deref(),
            Some("kind-oxikube")
        );
    }

    #[test]
    fn only_kind_contexts_are_allowed() {
        assert!(ensure_kind_context("kind-oxikube").is_ok());
        assert!(matches!(
            ensure_kind_context("prod-eu-1"),
            Err(IntegrationError::NotKindContext(_))
        ));
        assert!(TestNamespace::create("prod-eu-1").is_err());
    }

    #[test]
    fn namespace_names_are_unique_dns_labels() {
        let a = random_namespace_name();
        let b = random_namespace_name();
        assert_ne!(a, b);
        for n in [&a, &b] {
            assert!(n.starts_with(NAMESPACE_PREFIX));
            assert_eq!(n.len(), NAMESPACE_PREFIX.len() + 8);
            assert!(n.len() <= 63);
            assert!(
                n.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            );
        }
    }

    /// Runs only with a cluster (`OXIKUBE_TEST_CONTEXT=kind-oxikube`); skips otherwise.
    #[test]
    fn a_fresh_namespace_reports_its_events_as_text() {
        let Some(ctx) = test_context() else { return };
        let ns = TestNamespace::create(&ctx).expect("create namespace");
        let events = ns.events();
        assert!(!events.starts_with("(kubectl"), "{events}");
    }

    /// Runs only with a cluster (`OXIKUBE_TEST_CONTEXT=kind-oxikube`); skips otherwise.
    #[test]
    fn namespace_is_created_and_deleted_on_drop() {
        let Some(ctx) = test_context() else { return };
        let name;
        {
            let ns = TestNamespace::create(&ctx).expect("create namespace");
            name = ns.name().to_owned();
            kubectl(&ctx, &["get", "namespace", &name]).expect("namespace exists");
        }
        // Deletion is async (--wait=false): either gone or Terminating.
        let out = Command::new("kubectl")
            .args([
                "--context",
                &ctx,
                "get",
                "namespace",
                &name,
                "-o",
                "jsonpath={.status.phase}",
            ])
            .output()
            .unwrap();
        let phase = String::from_utf8_lossy(&out.stdout);
        assert!(
            !out.status.success() || phase == "Terminating",
            "phase: {phase}"
        );
    }
}
