//! [`CanIRequest`]: one can-i question in the words of `kubectl auth can-i`.

use oxikube_domain::access::AccessCheck;
use oxikube_domain::ids::Gvr;
use oxikube_domain::{AliasTarget, OxiError, OxiResult};
use serde_json::Value;

use super::subject::CanISubject;
use crate::search::aliases::AliasTable;

/// A question as the panel's fields or a tool call state it. Nothing here is resolved against a
/// cluster until [`to_check`](Self::to_check).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CanIRequest {
    /// `get`, `list`, `create`, `delete`, ... (`*` for any).
    pub verb: String,
    /// The resource as `kubectl` spells it: `pods`, `po`, `pods/exec`, `deployments.apps`.
    pub resource: String,
    /// The namespace; `None` or empty for a cluster-scoped resource or "all namespaces".
    pub namespace: Option<String>,
    /// One object's name; `None` for the kind as a whole.
    pub name: Option<String>,
    /// Who is asked as.
    pub subject: CanISubject,
}

impl CanIRequest {
    /// The request a tool call's `args` state.
    ///
    /// # Errors
    ///
    /// A validation error for a missing verb or resource or a malformed subject.
    pub fn from_args(args: &Value) -> OxiResult<Self> {
        let text = |key: &str| {
            args.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let subject = match args.get("subject") {
            None | Some(Value::Null) => CanISubject::Myself,
            Some(subject) => {
                let field = |key: &str| subject.get(key).and_then(Value::as_str);
                CanISubject::parse(
                    field("kind").unwrap_or_default(),
                    field("name").unwrap_or_default(),
                    field("namespace"),
                )?
            }
        };
        Ok(Self {
            verb: text("verb").ok_or_else(|| OxiError::validation("`verb` is required"))?,
            resource: text("resource")
                .ok_or_else(|| OxiError::validation("`resource` is required"))?,
            namespace: text("namespace"),
            name: text("name"),
            subject,
        })
    }

    /// The check this request asks, the resource resolved through the cluster's `aliases` (short
    /// names, plurals and Kinds), else read as `plural[.group]`.
    ///
    /// # Errors
    ///
    /// A validation error for an empty verb or resource.
    pub fn to_check(&self, aliases: &AliasTable) -> OxiResult<AccessCheck> {
        let verb = self.verb.trim().to_ascii_lowercase();
        if verb.is_empty() {
            return Err(OxiError::validation("the verb is empty"));
        }
        let (resource, subresource) = match self.resource.trim().split_once('/') {
            Some((resource, sub)) => (resource, Some(sub.trim()).filter(|s| !s.is_empty())),
            None => (self.resource.trim(), None),
        };
        if resource.is_empty() {
            return Err(OxiError::validation("the resource is empty"));
        }
        let gvr = match aliases.resolve(resource).target() {
            Some(AliasTarget::Gvr(gvr)) => gvr.clone(),
            _ => match resource.split_once('.') {
                Some((plural, group)) => Gvr::new(group, "", plural),
                None => Gvr::new("", "", resource),
            },
        };
        let mut check = AccessCheck::new(verb, gvr);
        if let Some(sub) = subresource {
            check = check.subresource(sub);
        }
        if let Some(namespace) = self.namespace.as_deref().filter(|s| !s.is_empty()) {
            check = check.namespace(namespace);
        }
        if let Some(name) = self.name.as_deref().filter(|s| !s.is_empty()) {
            check = check.name(name);
        }
        if let Some(identity) = self.subject.impersonation() {
            check = check.impersonating(identity);
        }
        Ok(check)
    }
}

/// The resource of `check` as `kubectl` spells it: `pods`, `pods/exec`, `deployments.apps`.
pub(super) fn resource_label(check: &AccessCheck) -> String {
    let mut label = check.gvr.resource.to_string();
    if !check.gvr.group.is_empty() {
        label.push('.');
        label.push_str(&check.gvr.group);
    }
    if let Some(sub) = &check.subresource {
        label.push('/');
        label.push_str(sub);
    }
    label
}
