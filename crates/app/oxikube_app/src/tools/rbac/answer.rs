//! [`CanIAnswer`]: the decision, its reason and which review produced it.

use oxikube_domain::access::{AccessCheck, AccessDecision};
use oxikube_domain::redact::redact;
use serde_json::{Value, json};

use super::request::resource_base;
use super::request::resource_label;
use super::subject::CanISubject;

/// Which review answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanIMethod {
    /// A `SelfSubjectAccessReview`: the current user.
    SelfSubjectAccessReview,
    /// A `SubjectAccessReview`: another subject, as the API server's authorizers see it.
    SubjectAccessReview,
}

impl CanIMethod {
    /// The review's Kubernetes kind.
    pub fn label(self) -> &'static str {
        match self {
            Self::SelfSubjectAccessReview => "SelfSubjectAccessReview",
            Self::SubjectAccessReview => "SubjectAccessReview",
        }
    }
}

/// The answer to one [`CanIRequest`](super::CanIRequest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanIAnswer {
    /// The check that was asked.
    pub check: AccessCheck,
    /// Who it was asked as.
    pub subject: CanISubject,
    /// Which review answered.
    pub method: CanIMethod,
    /// What the review decided.
    pub decision: AccessDecision,
}

impl CanIAnswer {
    /// `Allowed`, `Denied` or `Unknown`.
    pub fn outcome(&self) -> &'static str {
        match self.decision {
            AccessDecision::Allowed => "Allowed",
            AccessDecision::Denied { .. } => "Denied",
            AccessDecision::Unknown { .. } => "Unknown",
        }
    }

    /// The API's reason, redacted, or a plain sentence when it gave none.
    pub fn reason(&self) -> String {
        let reason = match &self.decision {
            AccessDecision::Allowed => None,
            AccessDecision::Denied { reason } => reason.as_deref(),
            AccessDecision::Unknown { reason } => Some(reason.as_str()),
        }
        .filter(|r| !r.is_empty());
        match (&self.decision, reason) {
            (_, Some(reason)) => redact(reason).into_owned(),
            (AccessDecision::Allowed, None) => "the authorizer allowed it".to_owned(),
            (_, None) => "no RBAC rule allows it".to_owned(),
        }
    }

    /// The question in one line: `delete pods in shop as ServiceAccount shop/bot`.
    pub fn question(&self) -> String {
        let mut text = format!("{} {}", self.check.verb, resource_label(&self.check));
        if let Some(name) = &self.check.name {
            text.push(' ');
            text.push_str(name);
        }
        match &self.check.namespace {
            Some(ns) => text.push_str(&format!(" in namespace {ns}")),
            None => text.push_str(" (cluster-wide)"),
        }
        if self.subject != CanISubject::Myself {
            text.push_str(&format!(" as {}", self.subject));
        }
        text
    }

    /// The equivalent `kubectl auth can-i` command.
    pub fn kubectl(&self) -> String {
        let mut cmd = format!(
            "kubectl auth can-i {} {}",
            self.check.verb,
            resource_base(&self.check)
        );
        if let Some(name) = &self.check.name {
            cmd.push('/');
            cmd.push_str(name);
        }
        if let Some(sub) = &self.check.subresource {
            cmd.push_str(" --subresource=");
            cmd.push_str(sub);
        }
        match &self.check.namespace {
            Some(ns) => cmd.push_str(&format!(" -n {ns}")),
            None => cmd.push_str(" --all-namespaces"),
        }
        cmd.push_str(&self.subject.kubectl_flags());
        cmd
    }

    /// The answer as text to copy: the question, the outcome, the reason and which review said so.
    pub fn to_text(&self) -> String {
        format!(
            "{}: {}\nreason: {}\nanswered by: {}\nkubectl: {}",
            self.question(),
            self.outcome(),
            self.reason(),
            self.method.label(),
            self.kubectl(),
        )
    }

    /// The answer as the structured result of `k8s.can_i`.
    pub fn to_json(&self) -> Value {
        json!({
            "allowed": self.decision.is_allowed(),
            "decision": self.outcome().to_ascii_lowercase(),
            "reason": self.reason(),
            "method": self.method.label(),
            "question": self.question(),
            "kubectl": self.kubectl(),
        })
    }
}
