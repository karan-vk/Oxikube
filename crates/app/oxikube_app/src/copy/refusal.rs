//! [`Refusal`]: why the `resource::CopyTo` handler turned a copy down before writing, attached to
//! its error so the service can tell the cases apart without parsing messages.

use std::error::Error as StdError;

use oxikube_domain::OxiError;
use oxikube_domain::ids::Gvk;

/// A precondition of a copy that the target does not meet.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum Refusal {
    /// The target does not serve the kind.
    #[error("{0}")]
    KindNotServed(String),
    /// The target serves the kind in other versions only.
    #[error("{0}")]
    VersionNotServed(String),
    /// The target namespace does not exist.
    #[error("namespace {0} does not exist on the target")]
    NamespaceMissing(String),
    /// The source object could not be read.
    #[error("{0}")]
    SourceUnavailable(String),
    /// The target name is not a valid DNS-1123 name.
    #[error("{0}")]
    InvalidName(String),
}

impl Refusal {
    /// The error for this refusal: a `Validation` error (the copy is wrong as asked) carrying the
    /// refusal as its source.
    pub(crate) fn into_error(self) -> OxiError {
        OxiError::validation(self.to_string()).with_source(self)
    }

    /// The refusal `error` carries, if it was made by [`into_error`](Self::into_error).
    pub(crate) fn of(error: &OxiError) -> Option<&Refusal> {
        StdError::source(error)?.downcast_ref::<Refusal>()
    }
}

/// How the API server validates the names of a kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameRule {
    /// A DNS-1123 label (Namespace, Service): lowercase alphanumerics and `-`, at most 63.
    Label,
    /// A DNS-1123 subdomain (most kinds): also `.`, at most 253.
    Subdomain,
    /// A path segment (RBAC kinds): any text without `/` or `%`, not `.` or `..`.
    PathSegment,
}

impl NameRule {
    /// The rule for names of `gvk`.
    pub(crate) fn of(gvk: &Gvk) -> Self {
        if gvk.is_core() && matches!(&*gvk.kind, "Namespace" | "Service") {
            Self::Label
        } else if &*gvk.group == "rbac.authorization.k8s.io" {
            Self::PathSegment
        } else {
            Self::Subdomain
        }
    }
}

/// Whether `name` is valid under `rule`: a DNS-1123 subdomain (lowercase alphanumerics, `-` and
/// `.`, starting and ending with an alphanumeric, at most 253 characters), the stricter label
/// (no dots, at most 63) or, for RBAC kinds, a path segment.
///
/// # Errors
///
/// The sentence that says what is wrong.
pub(crate) fn validate_name(name: &str, rule: NameRule) -> Result<(), String> {
    if rule == NameRule::PathSegment {
        return if name.is_empty() {
            Err("the name is empty".to_owned())
        } else if matches!(name, "." | "..") || name.contains(['/', '%']) {
            Err(format!("{name:?} is not a valid path segment name"))
        } else {
            Ok(())
        };
    }
    let label = rule == NameRule::Label;
    let (limit, what) = if label {
        (63, "DNS-1123 label")
    } else {
        (253, "DNS-1123 subdomain")
    };
    let alphanumeric = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    let valid_char = |c: char| alphanumeric(c) || c == '-' || (!label && c == '.');
    if name.is_empty() {
        return Err("the name is empty".to_owned());
    }
    if name.len() > limit {
        return Err(format!("the name is longer than {limit} characters"));
    }
    let edges = name.chars().next().is_some_and(alphanumeric)
        && name.chars().next_back().is_some_and(alphanumeric);
    if !name.chars().all(valid_char) || !edges {
        return Err(format!(
            "{name:?} is not a valid {what}: use lowercase letters, digits and '-'{}, starting \
             and ending with a letter or digit",
            if label { "" } else { " and '.'" }
        ));
    }
    Ok(())
}

#[cfg(test)]
mod name_tests {
    use super::*;

    #[test]
    fn the_rule_follows_the_kind() {
        let gvk = |group: &str, kind: &str| Gvk::new(group, "v1", kind);
        assert_eq!(NameRule::of(&gvk("", "Service")), NameRule::Label);
        assert_eq!(NameRule::of(&gvk("", "ConfigMap")), NameRule::Subdomain);
        assert_eq!(
            NameRule::of(&gvk("rbac.authorization.k8s.io", "ClusterRole")),
            NameRule::PathSegment
        );
        assert!(validate_name("system:aggregate-to-view", NameRule::PathSegment).is_ok());
        assert!(validate_name("a/b", NameRule::PathSegment).is_err());
        assert!(validate_name("..", NameRule::PathSegment).is_err());
        assert!(validate_name("Foo:bar", NameRule::Subdomain).is_err());
    }
}
