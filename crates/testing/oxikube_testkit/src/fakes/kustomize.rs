//! Fake [`KustomizePort`](oxikube_ports::KustomizePort) with scripted builds.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::kustomize::{BuildOutput, KustomizeError, KustomizeTool};
use oxikube_ports::KustomizePort;
use parking_lot::Mutex;

use crate::script::{CallLog, Script};

/// Queued responses for each [`FakeKustomizePort`] method.
#[derive(Debug, Default)]
pub struct KustomizeScripts {
    /// `build`.
    pub build: Script<BuildOutput>,
}

/// One call made on a [`FakeKustomizePort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KustomizeCall {
    /// `detect()`.
    Detect,
    /// `refresh()`.
    Refresh,
    /// `build(dir)`.
    Build(PathBuf),
}

/// Fake `KustomizePort`: the tool it "detects" is set by the test (`None` models a machine
/// without kustomize, which makes the capability false).
///
/// Fallbacks: `build` fails `Unsupported` ("not installed") when no tool is set, and otherwise
/// returns an empty output; a scripted response replaces the fallback for that call.
#[derive(Debug, Default)]
pub struct FakeKustomizePort {
    script: KustomizeScripts,
    calls: CallLog<KustomizeCall>,
    tool: Mutex<Option<KustomizeTool>>,
}

fake_plumbing!(FakeKustomizePort, KustomizeScripts, KustomizeCall);

impl FakeKustomizePort {
    /// A fake with no tool: nothing is installed.
    pub fn new() -> Self {
        Self::default()
    }

    /// A fake that detects `tool`.
    pub fn with_tool(tool: KustomizeTool) -> Self {
        let fake = Self::default();
        fake.set_tool(Some(tool));
        fake
    }

    /// Changes what `detect` and `refresh` find.
    pub fn set_tool(&self, tool: Option<KustomizeTool>) {
        *self.tool.lock() = tool;
    }
}

#[async_trait]
impl KustomizePort for FakeKustomizePort {
    async fn detect(&self) -> Option<KustomizeTool> {
        self.calls.record(KustomizeCall::Detect);
        self.tool.lock().clone()
    }

    async fn refresh(&self) -> Option<KustomizeTool> {
        self.calls.record(KustomizeCall::Refresh);
        self.tool.lock().clone()
    }

    async fn build(&self, dir: &Path) -> OxiResult<BuildOutput> {
        self.calls.record(KustomizeCall::Build(dir.to_path_buf()));
        self.script.build.next_or_else(|| {
            if self.tool.lock().is_some() {
                Ok(BuildOutput::default())
            } else {
                Err(KustomizeError::NotInstalled.into())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::kustomize::KustomizeToolKind;

    use super::*;

    fn tool() -> KustomizeTool {
        KustomizeTool {
            kind: KustomizeToolKind::Standalone,
            version: "v5.4.2".into(),
            path: "/usr/bin/kustomize".into(),
        }
    }

    #[test]
    fn nothing_installed_detects_none_and_cannot_build() {
        let fake = FakeKustomizePort::new();
        assert_eq!(block_on(fake.detect()), None);
        let error = block_on(fake.build(Path::new("/overlay"))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported);
    }

    #[test]
    fn a_tool_is_detected_and_builds_script() {
        let fake = FakeKustomizePort::with_tool(tool());
        assert_eq!(block_on(fake.detect()), Some(tool()));
        fake.script().build.push(Ok(BuildOutput {
            yaml: "kind: ConfigMap\n".into(),
            stderr: "warn".into(),
        }));
        let out = block_on(fake.build(Path::new("/overlay"))).unwrap();
        assert_eq!(out.stderr, "warn");
        fake.set_tool(None);
        assert_eq!(block_on(fake.refresh()), None);
        assert_eq!(
            fake.recorded_calls(),
            vec![
                KustomizeCall::Detect,
                KustomizeCall::Build("/overlay".into()),
                KustomizeCall::Refresh
            ]
        );
    }
}
