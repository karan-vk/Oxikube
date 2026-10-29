//! [`KustomizeService`]: builds a kustomize overlay and reads the result as manifests
//! (E20-S04).
//!
//! The build runs through the [`KustomizePort`] (a local process; the app layer never spawns
//! one). The output is manifest text, handed to the E20-S01 parser under the source label
//! `kustomize:<dir>`, so an error line points into the built output and the result says which
//! overlay it was built from. The documents are then planned and applied by the apply service
//! (E20-S03/S05) through `MutationGuard`; this module never writes to a cluster.
//!
//! The output can contain Secret values (`secretGenerator`): it is parsed and dropped, never
//! logged, and the `Debug` of [`KustomizeBuild`] names objects, not content.
//!
//! The capability: [`capabilities`](KustomizeService::capabilities) sets
//! [`Capabilities::KUSTOMIZE`] only while a tool exists, so a command that `needs` it is hidden
//! when neither `kustomize` nor `kubectl` is installed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::OxiResult;
use oxikube_domain::command::Capabilities;
use oxikube_domain::kustomize::{self, KustomizeTool};
use oxikube_domain::manifests::{ManifestDoc, ManifestError, SourceId, parse_documents};
use oxikube_ports::KustomizePort;

#[cfg(test)]
mod tests;

/// The label manifests built from `dir` are reported under (`kustomize:/path/to/overlay`).
pub fn source_id(dir: &Path) -> SourceId {
    SourceId::new(format!("kustomize:{}", dir.display()))
}

/// A successful build, read as manifests.
#[derive(Debug, Clone)]
pub struct KustomizeBuild {
    /// The overlay directory that was built ("built from" in the result panel).
    pub dir: PathBuf,
    /// The tool and version that built it, for the result panel and bug reports.
    pub tool: KustomizeTool,
    /// The objects, in output order.
    pub docs: Vec<ManifestDoc>,
    /// Output documents that could not be read (positions in the built output).
    pub errors: Vec<ManifestError>,
    /// What kustomize wrote to stderr while succeeding (warnings), redacted.
    pub stderr: String,
}

/// Detects kustomize and builds overlays with it.
#[derive(Clone)]
pub struct KustomizeService {
    port: Arc<dyn KustomizePort>,
}

impl KustomizeService {
    /// A service over `port`.
    pub fn new(port: Arc<dyn KustomizePort>) -> Self {
        Self { port }
    }

    /// The tool that would build, `None` when neither `kustomize` nor `kubectl kustomize`
    /// exists (the action is hidden).
    pub async fn tool(&self) -> Option<KustomizeTool> {
        self.port.detect().await
    }

    /// Looks for the tool again (after an install or a `kustomize.path` change).
    pub async fn refresh(&self) -> Option<KustomizeTool> {
        self.port.refresh().await
    }

    /// `base` with [`Capabilities::KUSTOMIZE`] following whether a tool exists.
    pub async fn capabilities(&self, base: Capabilities) -> Capabilities {
        kustomize::apply_to(base, self.tool().await.as_ref())
    }

    /// Builds the overlay in `dir` and parses the output. Dropping the future cancels the build
    /// and kills the process.
    ///
    /// # Errors
    ///
    /// The port's error: `Unsupported` without a tool or when the overlay needs exec plugins or
    /// Helm, `Validation` (message carries kustomize's stderr) when the build fails, `Timeout`.
    pub async fn build(&self, dir: &Path) -> OxiResult<KustomizeBuild> {
        let tool = self
            .port
            .detect()
            .await
            .ok_or(oxikube_domain::kustomize::KustomizeError::NotInstalled)?;
        let output = self.port.build(dir).await?;
        let (docs, errors) = parse_documents(&output.yaml, source_id(dir));
        Ok(KustomizeBuild {
            dir: dir.to_path_buf(),
            tool,
            docs,
            errors,
            stderr: output.stderr,
        })
    }
}
