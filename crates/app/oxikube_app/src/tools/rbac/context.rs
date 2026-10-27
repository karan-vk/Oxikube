//! `@rbac/sa/<namespace>/<name>` (E17-S08): a ServiceAccount's permissions as a block of text.
//!
//! The block lists the RoleBindings (in the namespace) and ClusterRoleBindings that name the
//! ServiceAccount, directly or through the groups every ServiceAccount is in, with the role each
//! points to, then a compact verb summary: `get list create patch delete` on a fixed set of common
//! resources, each answered by a `SubjectAccessReview` as the ServiceAccount. The two parts are
//! labelled (`# bindings` come from the objects, `# verbs` from the API server). Without permission
//! to create a `SubjectAccessReview` the verb part says so and the bindings still stand.
//!
//! Read-only, allowed on a read-only cluster. Lines pass through `oxikube_domain::redact`; the
//! body is cut to the caller's byte budget with a note.

use async_trait::async_trait;
use oxikube_domain::access::Impersonate;
use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::redact::redact;
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::{ContextProviderPort, ContextScope, Mention, ResourceReader};

use super::summary::{Summary, summarize};
use crate::session::ClusterSessionManager;
use crate::tools::metrics::session_for;

/// The mention prefix this provider owns: `@rbac/...`.
pub const RBAC_PREFIX: &str = "rbac";

/// Bindings read per kind; more are noted, not quoted.
const MAX_BINDINGS: u32 = 500;

/// Resolves `@rbac` mentions.
pub struct RbacContextProvider {
    sessions: ClusterSessionManager,
}

impl RbacContextProvider {
    /// A provider reading through `sessions`.
    pub fn new(sessions: ClusterSessionManager) -> Self {
        Self { sessions }
    }
}

impl std::fmt::Debug for RbacContextProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RbacContextProvider")
            .finish_non_exhaustive()
    }
}

/// One binding that names the ServiceAccount.
struct Bound {
    scope: String,
    binding: String,
    role: String,
    via: String,
}

#[async_trait]
impl ContextProviderPort for RbacContextProvider {
    fn mention_prefix(&self) -> &str {
        RBAC_PREFIX
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let (namespace, name) = match mention.path() {
            [kind, ns, name] if kind == "sa" && !ns.is_empty() && !name.is_empty() => {
                (ns.as_str(), name.as_str())
            }
            _ => {
                return Err(OxiError::validation(
                    "expected @rbac/sa/<namespace>/<name>, e.g. @rbac/sa/shop/builder",
                ));
            }
        };
        let session = session_for(&self.sessions, scope.cluster.as_ref())?;
        let mut lines = vec![
            format!("# cluster: {} ({})", session.title(), session.id()),
            format!("# subject: ServiceAccount {namespace}/{name}"),
        ];
        let mut notes = Vec::new();
        match session.resources() {
            Some(reader) => {
                let bound =
                    bindings(reader.as_ref(), session.id(), namespace, name, &mut notes).await;
                lines.push("# bindings: from RoleBindings and ClusterRoleBindings".to_owned());
                if bound.is_empty() {
                    lines.push("(no binding names this ServiceAccount)".to_owned());
                }
                for b in &bound {
                    lines.push(format!("{} {} -> {}{}", b.scope, b.binding, b.role, b.via));
                }
            }
            None => notes.push("the cluster is not connected; bindings are unavailable".into()),
        }
        let summary = match session.access() {
            Some(port) => {
                summarize(
                    port.as_ref(),
                    &Impersonate::service_account(namespace, name),
                    namespace,
                )
                .await
            }
            None => Summary {
                unavailable: Some("the cluster is not connected".to_owned()),
                ..Summary::default()
            },
        };
        lines.push(format!(
            "# verbs: SubjectAccessReview as the ServiceAccount, in namespace {namespace} (cluster-scoped kinds cluster-wide)"
        ));
        match &summary.unavailable {
            Some(why) => lines.push(format!("(unavailable: {})", redact(why))),
            None => {
                if summary.everything {
                    lines.push("* on *: allowed (cluster-admin level)".to_owned());
                }
                for row in &summary.rows {
                    let verbs = if row.allowed.is_empty() {
                        "none".to_owned()
                    } else {
                        row.allowed.join(" ")
                    };
                    lines.push(format!("{}: {verbs}", row.resource));
                }
            }
        }
        Ok(vec![build(
            format!("Permissions of ServiceAccount {namespace}/{name}"),
            lines,
            notes,
            scope.max_total_bytes,
        )])
    }
}

/// The block: lines until the budget, then notes, with a truncation note when cut.
fn build(title: String, lines: Vec<String>, notes: Vec<String>, cap: usize) -> ContextBlock {
    let cap = cap.min(MAX_CONTEXT_BLOCK_BYTES);
    let reserve = 160;
    let mut body = String::new();
    let mut shown = 0;
    for line in &lines {
        let line = redact(line);
        if body.len() + line.len() + 1 + reserve > cap {
            break;
        }
        body.push_str(&line);
        body.push('\n');
        shown += 1;
    }
    let cut = lines.len() - shown;
    if cut > 0 {
        body.push_str(&format!(
            "# note: {cut} lines left out to fit the size budget; use the k8s.can_i tool for specific questions\n"
        ));
    }
    for note in notes {
        body.push_str(&format!("# note: {}\n", redact(&note)));
    }
    let mut block = ContextBlock::bounded(title, "text/plain", body);
    block.truncated |= cut > 0;
    block
}

/// The bindings that name `namespace/name`, directly or via a ServiceAccount group.
async fn bindings(
    reader: &dyn ResourceReader,
    _cluster: &ClusterId,
    namespace: &str,
    name: &str,
    notes: &mut Vec<String>,
) -> Vec<Bound> {
    let groups = [
        "system:serviceaccounts".to_owned(),
        format!("system:serviceaccounts:{namespace}"),
    ];
    let mut found = Vec::new();
    let kinds = [
        (
            Gvk::new("rbac.authorization.k8s.io", "v1", "RoleBinding"),
            Some(namespace),
        ),
        (
            Gvk::new("rbac.authorization.k8s.io", "v1", "ClusterRoleBinding"),
            None,
        ),
    ];
    for (gvk, ns) in kinds {
        let options = oxikube_ports::ListOptions {
            limit: Some(MAX_BINDINGS),
            ..Default::default()
        };
        match reader.list(&gvk, ns, &options).await {
            Ok(page) => {
                if page.continue_token.is_some() {
                    notes.push(format!(
                        "more than {MAX_BINDINGS} {}s exist; only the first {MAX_BINDINGS} were read",
                        gvk.kind
                    ));
                }
                for item in &page.items {
                    if let Some(bound) = match_binding(item, ns, namespace, name, &groups) {
                        found.push(bound);
                    }
                }
            }
            Err(error) => notes.push(format!(
                "could not list {}s: {}",
                gvk.kind,
                redact(error.message())
            )),
        }
    }
    found
}

/// Whether `binding` names the ServiceAccount, and how.
fn match_binding(
    binding: &Resource,
    binding_ns: Option<&str>,
    namespace: &str,
    name: &str,
    groups: &[String],
) -> Option<Bound> {
    let subjects = binding.get("/subjects")?.as_array()?;
    let mut via = None;
    for subject in subjects.iter() {
        let field = |key: &str| subject.get(key).and_then(|v| v.as_str());
        match field("kind") {
            Some("ServiceAccount")
                if field("name") == Some(name)
                    && field("namespace").or(binding_ns) == Some(namespace) =>
            {
                via = Some(String::new());
                break;
            }
            Some("Group") if field("name").is_some_and(|g| groups.iter().any(|x| x == g)) => {
                via = Some(format!(
                    " (via group {})",
                    field("name").unwrap_or_default()
                ));
            }
            _ => {}
        }
    }
    let role = format!(
        "{} {}",
        binding.get_str("/roleRef/kind").unwrap_or("?"),
        binding.get_str("/roleRef/name").unwrap_or("?")
    );
    Some(Bound {
        scope: match binding_ns {
            Some(ns) => format!("RoleBinding {ns}/"),
            None => "ClusterRoleBinding".to_owned(),
        },
        binding: binding.name().to_owned(),
        role,
        via: via?,
    })
}
