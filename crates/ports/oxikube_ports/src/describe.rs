//! [`DescribePort`]: `kubectl describe`-style text for one object.
//!
//! # Adapter
//!
//! Implemented by `oxikube_describe`, which renders natively with deskribe and falls
//! back to shelling out to `kubectl describe` for kinds deskribe does not cover
//! (research 2.4). The result says which path produced it so the UI can label it.
//!
//! The call is read-only and runs off the UI thread.

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ResourceRef;

/// Which implementation produced a [`DescribeOutput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DescribeSource {
    /// Rendered in process (deskribe).
    Native,
    /// Produced by running `kubectl describe`.
    KubectlFallback,
}

/// The rendered description of one object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DescribeOutput {
    /// The description as plain text, in `kubectl describe` layout.
    pub text: String,
    /// How it was produced.
    pub source: DescribeSource,
}

/// Describes objects.
#[async_trait]
pub trait DescribePort: Send + Sync {
    /// Describes `target`, including its events. Errors with `NotFound` when the
    /// object does not exist.
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput>;
}
