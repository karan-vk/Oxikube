//! [`MetricRequest`], the built [`MetricQuery`] and [`MetricQueryError`].

use std::fmt;

use super::kind::{MetricKind, MetricScope, MetricUnit};
use super::target::MetricTarget;

/// An equality matcher added to every selector of a query: how a shared store (Mimir,
/// VictoriaMetrics) that holds several clusters is narrowed to one (`cluster="prod"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelMatch {
    name: String,
    value: String,
}

impl LabelMatch {
    /// A matcher for `name="value"`.
    ///
    /// # Errors
    ///
    /// [`MetricQueryError::InvalidLabelName`] when `name` is not a Prometheus label name
    /// (`[a-zA-Z_][a-zA-Z0-9_]*`): names, unlike values, cannot be escaped.
    pub fn new(
        name: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<Self, MetricQueryError> {
        let name = name.into();
        let mut chars = name.chars();
        let valid = chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(MetricQueryError::InvalidLabelName(name));
        }
        Ok(Self {
            name,
            value: value.into(),
        })
    }

    /// The label name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The label value (unescaped).
    pub fn value(&self) -> &str {
        &self.value
    }
}

/// What a graph asks for: one [`MetricKind`] of one [`MetricTarget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricRequest {
    /// The object.
    pub target: MetricTarget,
    /// What to measure.
    pub kind: MetricKind,
    /// Matchers added to every selector (see [`LabelMatch`]). Usually empty.
    pub extra: Vec<LabelMatch>,
}

impl MetricRequest {
    /// A request without extra matchers.
    pub fn new(target: MetricTarget, kind: MetricKind) -> Self {
        Self {
            target,
            kind,
            extra: Vec::new(),
        }
    }

    /// Adds a matcher to every selector of the query.
    #[must_use]
    pub fn with_label(mut self, label: LabelMatch) -> Self {
        self.extra.push(label);
        self
    }

    /// The scope of the target.
    pub const fn scope(&self) -> MetricScope {
        self.target.scope()
    }

    /// Checks the request can be built: the combination exists and no name is empty.
    ///
    /// # Errors
    ///
    /// [`MetricQueryError::Unsupported`], [`MetricQueryError::EmptyName`] or
    /// [`MetricQueryError::NoPods`].
    pub fn validate(&self) -> Result<(), MetricQueryError> {
        let scope = self.scope();
        if !scope.supports(self.kind) {
            return Err(MetricQueryError::Unsupported {
                scope,
                kind: self.kind,
            });
        }
        let empty = |value: &str, what: &'static str| {
            if value.is_empty() {
                Err(MetricQueryError::EmptyName(what))
            } else {
                Ok(())
            }
        };
        match &self.target {
            MetricTarget::Cluster => {}
            MetricTarget::Node(name) => empty(name, "node")?,
            MetricTarget::Namespace(namespace) => empty(namespace, "namespace")?,
            MetricTarget::Pod { namespace, pods } => {
                empty(namespace, "namespace")?;
                if pods.names().is_empty() {
                    return Err(MetricQueryError::NoPods);
                }
                for name in pods.names() {
                    empty(name, "pod")?;
                }
            }
            MetricTarget::Container {
                namespace,
                pod,
                container,
            } => {
                empty(namespace, "namespace")?;
                empty(pod, "pod")?;
                if let Some(container) = container {
                    empty(container, "container")?;
                }
            }
            MetricTarget::Pvc { namespace, name } => {
                empty(namespace, "namespace")?;
                empty(name, "persistent volume claim")?;
            }
            MetricTarget::Ingress { namespace, name } => {
                empty(namespace, "namespace")?;
                empty(name, "ingress")?;
            }
        }
        Ok(())
    }
}

/// A built query: the PromQL text and what the caller needs to read its result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricQuery {
    /// The PromQL expression, ready for an instant or range query.
    pub promql: String,
    /// The label that names each series of the result (`pod`, `container`), or `None` when the
    /// result is a single series.
    pub series_label: Option<&'static str>,
    /// The unit of the values.
    pub unit: MetricUnit,
}

/// Why a [`MetricRequest`] could not become a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetricQueryError {
    /// No query exists for this kind at this scope (see [`MetricScope::supports`]).
    Unsupported {
        /// The scope asked for.
        scope: MetricScope,
        /// The kind asked for.
        kind: MetricKind,
    },
    /// A name the target needs is empty.
    EmptyName(&'static str),
    /// A [`PodSelector::Any`](super::PodSelector::Any) without pods.
    NoPods,
    /// A [`LabelMatch`] name that is not a valid label name.
    InvalidLabelName(String),
    /// The `rate()` window is zero.
    EmptyWindow,
}

impl fmt::Display for MetricQueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetricQueryError::Unsupported { scope, kind } => {
                write!(f, "no {kind} query exists at {scope} scope")
            }
            MetricQueryError::EmptyName(what) => write!(f, "the {what} name is empty"),
            MetricQueryError::NoPods => f.write_str("no pod names were given"),
            MetricQueryError::InvalidLabelName(name) => {
                write!(f, "`{name}` is not a valid label name")
            }
            MetricQueryError::EmptyWindow => f.write_str("the rate window is zero"),
        }
    }
}

impl std::error::Error for MetricQueryError {}
