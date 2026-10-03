//! Context providers: resolve `@`-mentions into [`ContextBlock`]s.
//!
//! A *mention* is what the user types in an agent prompt to attach cluster
//! context: `@pod/default/web-0`, `@logs/default/web-0`, `@events/default`.
//! The first segment is the [`MentionPrefix`]; every provider claims one prefix
//! ([`ContextProviderPort::mention_prefix`]) and `oxikube_app::ContextRegistry`
//! routes a parsed [`Mention`] to the provider that owns it.
//!
//! Integrations contribute providers through
//! [`IntegrationPort::context_providers`](crate::integration::IntegrationPort::context_providers),
//! for example `@argo/<app>`.
//!
//! This module also holds [`ContentPart`], the content shape shared by agent
//! prompts, agent message chunks and tool results (MCP and ACP use the same
//! text / image / embedded-resource / resource-link set).
//!
//! # Budget and redaction
//!
//! [`ContextBlock::bounded`] caps one block at
//! [`MAX_CONTEXT_BLOCK_BYTES`];
//! [`ContextScope::max_total_bytes`] caps everything one `resolve` call returns.
//! Providers redact Secret data and tokens *before* building a block
//! (non-negotiable 5); the domain does not redact.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::agent::{ContextBlock, MAX_CONTEXT_BLOCK_BYTES};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use serde::{Deserialize, Serialize};

/// The leading segment of a [`Mention`], for example `pod` or `logs`.
///
/// Lowercase ASCII: `[a-z][a-z0-9_-]*`, at most 32 bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MentionPrefix(Arc<str>);

/// Longest [`MentionPrefix`], in bytes.
pub const MAX_MENTION_PREFIX_BYTES: usize = 32;

impl MentionPrefix {
    /// Whether `s` is a valid prefix.
    pub fn is_valid(s: &str) -> bool {
        let b = s.as_bytes();
        !b.is_empty()
            && b.len() <= MAX_MENTION_PREFIX_BYTES
            && b[0].is_ascii_lowercase()
            && b.iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
    }

    /// Validates and wraps `s`.
    pub fn new(s: &str) -> OxiResult<Self> {
        if Self::is_valid(s) {
            Ok(Self(Arc::from(s)))
        } else {
            Err(OxiError::validation(format!(
                "invalid mention prefix {s:?}: expected [a-z][a-z0-9_-]* of at most {MAX_MENTION_PREFIX_BYTES} bytes"
            )))
        }
    }

    /// The prefix text, without the `@`.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MentionPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for MentionPrefix {
    type Error = OxiError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<MentionPrefix> for String {
    fn from(value: MentionPrefix) -> Self {
        value.0.to_string()
    }
}

/// A parsed `@prefix/segment/segment` mention.
///
/// Segments are what the provider interprets: `@pod/default/web-0` is prefix
/// `pod` with path `["default", "web-0"]`; `@events/default` has path
/// `["default"]`; a bare `@cluster` has an empty path. Segments are non-empty
/// and contain no whitespace, `/` or `@`. `Display` round-trips
/// [`Mention::parse`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Mention {
    prefix: MentionPrefix,
    path: Vec<String>,
}

impl Mention {
    /// Parses text such as `@logs/default/web-0` (the leading `@` is required).
    ///
    /// Fails with a validation error on a missing `@`, a bad prefix or an empty
    /// or malformed segment (`@pod//x`, `@pod/x/`).
    pub fn parse(text: &str) -> OxiResult<Self> {
        let rest = text
            .strip_prefix('@')
            .ok_or_else(|| OxiError::validation(format!("mention {text:?} must start with '@'")))?;
        let mut parts = rest.split('/');
        let prefix = MentionPrefix::new(parts.next().unwrap_or_default())?;
        let mut path = Vec::new();
        for seg in parts {
            if seg.is_empty() || seg.chars().any(|c| c.is_whitespace() || c == '@') {
                return Err(OxiError::validation(format!(
                    "mention {text:?} has an empty or malformed segment"
                )));
            }
            path.push(seg.to_owned());
        }
        Ok(Self { prefix, path })
    }

    /// The prefix naming the owning provider.
    pub fn prefix(&self) -> &MentionPrefix {
        &self.prefix
    }

    /// The segments after the prefix.
    pub fn path(&self) -> &[String] {
        &self.path
    }
}

impl fmt::Display for Mention {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.prefix)?;
        for seg in &self.path {
            write!(f, "/{seg}")?;
        }
        Ok(())
    }
}

/// What a provider may use to resolve a mention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextScope {
    /// The active cluster, when the thread is bound to one.
    pub cluster: Option<ClusterId>,
    /// The namespace to assume when the mention omits it, when the thread has one.
    pub default_namespace: Option<String>,
    /// Budget for the combined size of the returned block bodies, in bytes.
    /// Providers drop or truncate blocks to fit and set
    /// [`ContextBlock::truncated`].
    pub max_total_bytes: usize,
}

impl ContextScope {
    /// A scope with no cluster, no default namespace and a budget of one full block.
    pub fn new() -> Self {
        Self {
            cluster: None,
            default_namespace: None,
            max_total_bytes: MAX_CONTEXT_BLOCK_BYTES,
        }
    }

    /// Binds the scope to a cluster.
    #[must_use]
    pub fn with_cluster(mut self, cluster: ClusterId) -> Self {
        self.cluster = Some(cluster);
        self
    }

    /// Sets the default namespace.
    #[must_use]
    pub fn with_default_namespace(mut self, namespace: impl Into<String>) -> Self {
        self.default_namespace = Some(namespace.into());
        self
    }

    /// Sets the total byte budget.
    #[must_use]
    pub fn with_max_total_bytes(mut self, bytes: usize) -> Self {
        self.max_total_bytes = bytes;
        self
    }
}

impl Default for ContextScope {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves mentions with one prefix into context blocks. Read-only.
///
/// Implemented per feature (`oxikube_app` for resource, logs and events
/// providers; integrations for their own) and registered in
/// `oxikube_app::ContextRegistry`.
///
/// # Effects
///
/// Read-only. A provider never mutates the cluster or any store.
///
/// # Errors
///
/// [`resolve`](Self::resolve) returns [`NotFound`](oxikube_domain::ErrorKind::NotFound)
/// for a missing target, [`Validation`](oxikube_domain::ErrorKind::Validation) for a
/// malformed mention path, and [`Forbidden`](oxikube_domain::ErrorKind::Forbidden) when
/// RBAC denies the underlying read. Connection failures surface as
/// [`Network`](oxikube_domain::ErrorKind::Network) or
/// [`Timeout`](oxikube_domain::ErrorKind::Timeout) (retryable).
#[async_trait]
pub trait ContextProviderPort: Send + Sync {
    /// The prefix this provider owns, for example `"pod"` or `"logs"`. Must
    /// satisfy [`MentionPrefix::is_valid`].
    fn mention_prefix(&self) -> &str;

    /// Whether this provider owns `mention`. The default compares prefixes.
    fn handles(&self, mention: &Mention) -> bool {
        mention.prefix().as_str() == self.mention_prefix()
    }

    /// Resolves `mention` into zero or more blocks.
    ///
    /// Returns [`NotFound`](oxikube_domain::ErrorKind::NotFound) when the
    /// target does not exist and [`Validation`](oxikube_domain::ErrorKind::Validation)
    /// when the path has the wrong shape. Blocks are redacted and bounded
    /// (see the module docs) and fit `scope.max_total_bytes` together.
    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>>;
}

/// One piece of content in an agent prompt, message chunk or tool result.
///
/// Mirrors the content set MCP and ACP share (text, image, embedded resource,
/// resource link) without copying either protocol's types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ContentPart {
    /// Plain text (Markdown for agent messages).
    Text {
        /// The text.
        text: String,
    },
    /// An image, base64 encoded.
    Image {
        /// MIME type, for example `image/png`.
        mime: String,
        /// Base64 data.
        data: String,
    },
    /// Embedded cluster context, usually resolved from a mention.
    Resource(ContextBlock),
    /// A reference to a resource the receiver may fetch itself.
    ResourceLink {
        /// The URI, for example `oxikube://pod/default/web-0`.
        uri: String,
        /// A display name.
        name: String,
    },
}

impl ContentPart {
    /// A text part.
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    /// The text of a [`ContentPart::Text`].
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}

impl From<ContextBlock> for ContentPart {
    fn from(block: ContextBlock) -> Self {
        Self::Resource(block)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::ErrorKind;
    use proptest::prelude::*;

    #[test]
    fn parses_resource_logs_and_events_mentions() {
        let m = Mention::parse("@pod/default/web-0").unwrap();
        assert_eq!(m.prefix().as_str(), "pod");
        assert_eq!(m.path(), ["default", "web-0"]);
        assert_eq!(m.to_string(), "@pod/default/web-0");

        let m = Mention::parse("@logs/default/web-0").unwrap();
        assert_eq!(m.prefix().as_str(), "logs");
        let m = Mention::parse("@events/default").unwrap();
        assert_eq!(m.path(), ["default"]);
        let m = Mention::parse("@cluster").unwrap();
        assert!(m.path().is_empty());
        assert_eq!(m.to_string(), "@cluster");
    }

    #[test]
    fn rejects_malformed_mentions() {
        for bad in [
            "",
            "pod/default/x",
            "@",
            "@Pod/x",
            "@1pod",
            "@pod//x",
            "@pod/x/",
            "@pod/a b",
            "@pod/a@b",
            "@pod x",
            &format!("@{}", "a".repeat(MAX_MENTION_PREFIX_BYTES + 1)),
        ] {
            let err = Mention::parse(bad).unwrap_err();
            assert_eq!(err.kind(), ErrorKind::Validation, "{bad:?}");
        }
    }

    #[test]
    fn prefix_serde_validates() {
        let p: MentionPrefix = serde_json::from_str("\"logs\"").unwrap();
        assert_eq!(serde_json::to_string(&p).unwrap(), "\"logs\"");
        assert!(serde_json::from_str::<MentionPrefix>("\"Bad\"").is_err());
    }

    #[test]
    fn scope_builders_and_default_budget() {
        let s = ContextScope::default();
        assert_eq!(s.max_total_bytes, MAX_CONTEXT_BLOCK_BYTES);
        let s = s
            .with_default_namespace("kube-system")
            .with_max_total_bytes(10);
        assert_eq!(s.default_namespace.as_deref(), Some("kube-system"));
        assert_eq!(s.max_total_bytes, 10);
    }

    #[test]
    fn content_part_serde_round_trip() {
        let parts = vec![
            ContentPart::text("hello"),
            ContentPart::Image {
                mime: "image/png".into(),
                data: "AAAA".into(),
            },
            ContentPart::from(ContextBlock::text("Pod web-0", "kind: Pod")),
            ContentPart::ResourceLink {
                uri: "oxikube://pod/default/web-0".into(),
                name: "web-0".into(),
            },
        ];
        let json = serde_json::to_value(&parts).unwrap();
        assert_eq!(json[0]["type"], "text");
        assert_eq!(json[2]["type"], "resource");
        let back: Vec<ContentPart> = serde_json::from_value(json).unwrap();
        assert_eq!(back, parts);
        assert_eq!(parts[0].as_text(), Some("hello"));
        assert_eq!(parts[1].as_text(), None);
    }

    proptest! {
        #[test]
        fn parse_display_round_trips(
            prefix in "[a-z][a-z0-9_-]{0,31}",
            path in proptest::collection::vec("[A-Za-z0-9._:-]{1,20}", 0..4),
        ) {
            let text = format!("@{prefix}{}", path.iter().map(|s| format!("/{s}")).collect::<String>());
            let m = Mention::parse(&text).unwrap();
            prop_assert_eq!(m.to_string(), text);
            prop_assert_eq!(m.path().len(), path.len());
        }

        #[test]
        fn arbitrary_text_never_panics(s in any::<String>()) {
            if let Ok(m) = Mention::parse(&s) {
                prop_assert_eq!(Mention::parse(&m.to_string()).unwrap(), m);
            }
        }
    }
}
