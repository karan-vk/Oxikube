//! [`KubeDrain`]: the [`DrainPort`] over this algorithm.

use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::drain::DrainOptions;
use oxikube_ports::{DrainPort, DrainStream, ResourceReader, ResourceWriter};

use super::run::drain_with;

/// The `kubectl drain` algorithm as a [`DrainPort`]: [`drain_with`] over the reader and writer
/// each call is given. Stateless; one serves every cluster.
///
/// The writer the app passes is the guard's (`Mutation`), which wraps the session's
/// [`KubeResources`](crate::KubeResources): evictions go through its unretried client (see
/// [`with_lazy_unretried_client`](crate::KubeResources::with_lazy_unretried_client), which the
/// connector sets), so a PodDisruptionBudget's 429 reaches the drain at once and shows as
/// "blocked" while the drain backs off.
#[derive(Debug, Clone, Copy, Default)]
pub struct KubeDrain;

impl DrainPort for KubeDrain {
    fn drain(
        &self,
        reader: Arc<dyn ResourceReader>,
        writer: Arc<dyn ResourceWriter>,
        node: &str,
        options: DrainOptions,
    ) -> DrainStream {
        drain_with(reader, writer, node, options).boxed()
    }
}
