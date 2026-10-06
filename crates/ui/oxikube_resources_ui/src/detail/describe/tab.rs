//! The Describe tab's state and the view's methods over it.

use std::sync::Arc;

use gpui::{Context, Entity, Task};
use oxikube_domain::command::Command;
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{DescribeOutput, DescribeSource};
use oxikube_runtime::spawn_kube;
use oxikube_ui::editor::EditorState;

use crate::detail::view::DetailView;

/// How the Describe tab stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescribeState {
    /// Not asked for yet (the tab was never shown).
    Idle,
    /// The text is being read (a refresh keeps the previous text on screen meanwhile).
    Loading,
    /// The text is there.
    Ready {
        /// Which backend rendered it.
        source: DescribeSource,
    },
    /// The last attempt failed.
    Failed {
        /// The error's kind (`Unsupported` for a kind neither backend covers).
        kind: ErrorKind,
        /// What went wrong, redacted by the adapter.
        message: String,
    },
}

/// A describe text read, with the number of the answer that brought it (the editor is given a
/// text again only when this changes).
pub(in crate::detail) struct DescribeText {
    pub(in crate::detail) text: Arc<str>,
    pub(in crate::detail) source: DescribeSource,
    pub(in crate::detail) serial: u64,
}

/// The Describe tab of a [`DetailView`].
pub(in crate::detail) struct DescribeTab {
    pub(in crate::detail) started: bool,
    pub(in crate::detail) state: DescribeState,
    /// The last text read, kept while a refresh is in flight or failed.
    pub(in crate::detail) output: Option<DescribeText>,
    /// Counts requests: only the answer to the latest one is applied.
    pub(in crate::detail) generation: u64,
    /// The `serial` of the text the editor holds.
    pub(in crate::detail) pushed: Option<u64>,
    pub(in crate::detail) editor: Option<Entity<EditorState>>,
    /// The request in flight; dropping it cancels it (and `kubectl`).
    pub(in crate::detail) task: Option<Task<()>>,
}

impl Default for DescribeTab {
    fn default() -> Self {
        Self {
            started: false,
            state: DescribeState::Idle,
            output: None,
            generation: 0,
            pushed: None,
            editor: None,
            task: None,
        }
    }
}

impl DetailView {
    /// How the Describe tab stands.
    pub fn describe_state(&self) -> &DescribeState {
        &self.describe.state
    }

    /// The describe text last read.
    pub fn describe_text(&self) -> Option<&str> {
        self.describe.output.as_ref().map(|output| &*output.text)
    }

    /// Starts the describe the first time the tab is shown.
    pub(in crate::detail) fn start_describe(&mut self, cx: &mut Context<Self>) {
        if self.describe.started {
            return;
        }
        self.describe.started = true;
        self.refresh_describe(cx);
    }

    /// The refresh button (and the Retry button): sends `resource::RefreshDescribe`.
    pub fn request_refresh_describe(&mut self, cx: &mut Context<Self>) {
        let command = Command::ResourceRefreshDescribe {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Reads the describe text again (`resource::RefreshDescribe`). An earlier request still in
    /// flight is cancelled. Starts the first read when the tab was never shown.
    pub fn refresh_describe(&mut self, cx: &mut Context<Self>) {
        self.describe.started = true;
        let Some(port) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.describe())
        else {
            self.describe.task = None;
            self.describe.state = DescribeState::Failed {
                kind: ErrorKind::Network,
                message: "The cluster is not connected.".to_owned(),
            };
            cx.notify();
            return;
        };
        self.describe.generation += 1;
        let generation = self.describe.generation;
        self.describe.state = DescribeState::Loading;
        let target = self.target.clone();
        let work = spawn_kube(cx, async move { port.describe(&target).await });
        self.describe.task = Some(cx.spawn(async move |this, cx| {
            let result = match work.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |view, cx| view.described(generation, result, cx))
                .ok();
        }));
        cx.notify();
    }

    /// Applies an answer, unless a newer request replaced the one it answers.
    fn described(
        &mut self,
        generation: u64,
        result: OxiResult<DescribeOutput>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.describe.generation {
            return;
        }
        match result {
            Ok(output) => {
                self.describe.state = DescribeState::Ready {
                    source: output.source,
                };
                let serial = self
                    .describe
                    .output
                    .as_ref()
                    .map_or(1, |old| old.serial + 1);
                self.describe.output = Some(DescribeText {
                    text: Arc::from(output.text),
                    source: output.source,
                    serial,
                });
            }
            Err(error) => {
                tracing::debug!(%error, target = %self.target, "describe failed");
                self.describe.state = DescribeState::Failed {
                    kind: error.kind(),
                    message: error.message().to_owned(),
                };
            }
        }
        cx.notify();
    }
}
