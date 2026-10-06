//! [`Describer`]: the `DescribePort` the app gets, choosing a backend per call.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DescribeOutput, DescribePort};

use crate::preference::{Backend, DescribePreference};

/// Describes objects with the backend the [`DescribePreference`] names at the time of the call,
/// so a settings change applies to the next describe without reconnecting.
///
/// With [`Backend::Auto`] the native renderer goes first; only an
/// [`Unsupported`](ErrorKind::Unsupported) answer (a kind deskribe does not cover) moves on to
/// `kubectl`. Any other failure (not found, forbidden, network) is the answer as it is: `kubectl`
/// would meet the same one.
pub struct Describer {
    native: Arc<dyn DescribePort>,
    kubectl: Arc<dyn DescribePort>,
    preference: DescribePreference,
}

impl std::fmt::Debug for Describer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Describer").finish_non_exhaustive()
    }
}

impl Describer {
    /// A describer over the two backends.
    pub fn new(
        native: Arc<dyn DescribePort>,
        kubectl: Arc<dyn DescribePort>,
        preference: DescribePreference,
    ) -> Self {
        Self {
            native,
            kubectl,
            preference,
        }
    }
}

#[async_trait]
impl DescribePort for Describer {
    async fn describe(&self, target: &ResourceRef) -> OxiResult<DescribeOutput> {
        match self.preference.get().backend {
            Backend::Native => self.native.describe(target).await,
            Backend::Kubectl => self.kubectl.describe(target).await,
            Backend::Auto => match self.native.describe(target).await {
                Err(native) if native.kind() == ErrorKind::Unsupported => {
                    match self.kubectl.describe(target).await {
                        Err(kubectl) if kubectl.kind() == ErrorKind::Unsupported => {
                            Err(OxiError::unsupported(format!(
                                "{}; {}",
                                native.message(),
                                kubectl.message()
                            )))
                        }
                        other => other,
                    }
                }
                other => other,
            },
        }
    }
}
