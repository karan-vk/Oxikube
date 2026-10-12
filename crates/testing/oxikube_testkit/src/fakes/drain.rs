//! Fake [`DrainPort`](oxikube_ports::DrainPort): scripted progress, writes through the writer it
//! is given.

use std::collections::VecDeque;
use std::sync::Arc;

use futures::StreamExt as _;
use oxikube_domain::OxiResult;
use oxikube_domain::drain::{DrainOptions, DrainProgress, DrainSummary};
use oxikube_domain::ids::Gvk;
use oxikube_ports::{
    DeleteOptions, DrainPort, DrainStream, Patch, ResourceReader, ResourceWriter, WriteOptions,
};
use serde_json::json;

use crate::script::{CallLog, Script};

/// One scripted drain: the steps it reports and whether it then hangs (never finishes, for
/// cancel tests).
#[derive(Debug, Default)]
pub struct DrainRun {
    /// The items of the stream, in order. A `Cordoned` step patches the node through the writer
    /// first, an `Evicted` step evicts its pod through it first; a write that fails ends the
    /// stream with that error instead, as the real algorithm would.
    pub steps: Vec<OxiResult<DrainProgress>>,
    /// After the last step the stream stays pending instead of ending.
    pub then_hold: bool,
}

impl DrainRun {
    /// A run of `steps` that ends after them.
    pub fn of(steps: impl IntoIterator<Item = DrainProgress>) -> Self {
        Self {
            steps: steps.into_iter().map(Ok).collect(),
            then_hold: false,
        }
    }

    /// The same run, hanging after its last step.
    #[must_use]
    pub fn hold(mut self) -> Self {
        self.then_hold = true;
        self
    }
}

/// Queued responses for each [`FakeDrainPort`] method.
#[derive(Debug, Default)]
pub struct DrainScripts {
    /// `drain`: each call pops one run; a scripted error is a stream of that one error.
    pub drain: Script<DrainRun>,
}

/// One call made on a [`FakeDrainPort`].
#[derive(Debug, Clone, PartialEq)]
pub enum DrainCall {
    /// `drain(reader, writer, node, options)`.
    Drain {
        /// The node.
        node: String,
        /// The options it was asked with.
        options: DrainOptions,
    },
}

/// Fake `DrainPort` (E12-S10). Records each call; replays the scripted [`DrainRun`].
///
/// Fallback when nothing is scripted: a drain of a node with no pods (`Planned` with nothing,
/// `Cordoned { already: false }` after the cordon patch through the writer, `Finished`), or with
/// `dry_run` just `Planned` and a dry-run `Finished` and no write. Nothing sleeps and nothing is
/// spawned; the caller's task drives the stream.
#[derive(Debug, Default)]
pub struct FakeDrainPort {
    script: DrainScripts,
    calls: CallLog<DrainCall>,
}

fake_plumbing!(FakeDrainPort, DrainScripts, DrainCall);

impl FakeDrainPort {
    /// A fake with nothing scripted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `run` for the next `drain` call.
    pub fn push(&self, run: DrainRun) -> &Self {
        self.script.drain.push_ok(run);
        self
    }

    fn fallback(node: &str, options: &DrainOptions) -> DrainRun {
        let summary = DrainSummary {
            node: node.to_owned(),
            evicted: Vec::new(),
            skipped: Vec::new(),
            failed: Vec::new(),
            dry_run: options.dry_run,
        };
        let planned = DrainProgress::Planned {
            evict: Vec::new(),
            skipped: Vec::new(),
        };
        if options.dry_run {
            DrainRun::of([planned, DrainProgress::Finished(summary)])
        } else {
            DrainRun::of([
                planned,
                DrainProgress::Cordoned { already: false },
                DrainProgress::Finished(summary),
            ])
        }
    }
}

/// What the replay holds between items.
struct Replay {
    steps: VecDeque<OxiResult<DrainProgress>>,
    hold: bool,
    writer: Arc<dyn ResourceWriter>,
    node: String,
    done: bool,
}

impl Replay {
    /// The next item, after the write it stands for.
    async fn next(&mut self) -> Option<OxiResult<DrainProgress>> {
        if self.done {
            return None;
        }
        let Some(step) = self.steps.pop_front() else {
            if self.hold {
                futures::future::pending::<()>().await;
            }
            return None;
        };
        let write = match &step {
            Ok(DrainProgress::Cordoned { already: false }) => {
                let patch = Patch::merge(json!({"spec": {"unschedulable": true}}));
                self.writer
                    .patch(
                        &Gvk::new("", "v1", "Node"),
                        None,
                        &self.node,
                        &patch,
                        &WriteOptions::default(),
                    )
                    .await
                    .map(drop)
            }
            Ok(DrainProgress::Evicted { pod }) => {
                self.writer
                    .evict(&pod.namespace, &pod.name, &DeleteOptions::default())
                    .await
            }
            _ => Ok(()),
        };
        if let Err(error) = write {
            self.done = true;
            return Some(Err(error));
        }
        if step.is_err() {
            self.done = true;
        }
        Some(step)
    }
}

impl DrainPort for FakeDrainPort {
    fn drain(
        &self,
        _reader: Arc<dyn ResourceReader>,
        writer: Arc<dyn ResourceWriter>,
        node: &str,
        options: DrainOptions,
    ) -> DrainStream {
        self.calls.record(DrainCall::Drain {
            node: node.to_owned(),
            options: options.clone(),
        });
        let run = match self.script.drain.pop() {
            Some(Ok(run)) => run,
            Some(Err(error)) => DrainRun {
                steps: vec![Err(error)],
                then_hold: false,
            },
            None => Self::fallback(node, &options),
        };
        let replay = Replay {
            steps: run.steps.into(),
            hold: run.then_hold,
            writer,
            node: node.to_owned(),
            done: false,
        };
        futures::stream::unfold(replay, |mut replay| async move {
            let item = replay.next().await?;
            Some((item, replay))
        })
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_domain::drain::PodRef;
    use oxikube_domain::{ErrorKind, OxiError};

    use super::*;
    use crate::FakeResourcePort;

    fn port() -> Arc<FakeResourcePort> {
        Arc::new(FakeResourcePort::new())
    }

    fn pod(name: &str) -> PodRef {
        PodRef {
            namespace: "default".into(),
            name: name.into(),
            uid: None,
        }
    }

    #[test]
    fn the_fallback_cordons_through_the_writer_and_finishes() {
        let fake = FakeDrainPort::new();
        let cluster = port();
        cluster.insert(
            oxikube_domain::Resource::from_json(json!({
                "apiVersion": "v1", "kind": "Node", "metadata": {"name": "n1"},
            }))
            .unwrap(),
        );
        let steps: Vec<_> = block_on(
            fake.drain(
                cluster.clone(),
                cluster.clone(),
                "n1",
                DrainOptions::default(),
            )
            .collect::<Vec<_>>(),
        );
        assert_eq!(steps.len(), 3);
        assert!(matches!(steps[2], Ok(DrainProgress::Finished(_))));
        assert_eq!(cluster.mutating_calls().len(), 1, "the cordon patch");
        assert_eq!(
            fake.recorded_calls(),
            [DrainCall::Drain {
                node: "n1".into(),
                options: DrainOptions::default(),
            }]
        );
    }

    #[test]
    fn a_failed_write_ends_the_replay_with_its_error() {
        let fake = FakeDrainPort::new();
        fake.push(DrainRun::of([
            DrainProgress::Cordoned { already: true },
            DrainProgress::Evicted { pod: pod("a") },
            DrainProgress::Gone { pod: pod("a") },
        ]));
        let cluster = port();
        cluster
            .script()
            .evict
            .push_err(OxiError::forbidden("read-only"));
        let steps: Vec<_> = block_on(
            fake.drain(cluster.clone(), cluster, "n1", DrainOptions::default())
                .collect::<Vec<_>>(),
        );
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[1].as_ref().unwrap_err().kind(), ErrorKind::Forbidden);
    }
}
