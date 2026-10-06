//! [`SettingsPrefsWriter`]: the posture commands' way into `settings.json` (E06-S09).
//!
//! The command handlers in `oxikube_app` run on whatever task dispatched the command and know
//! nothing of GPUI; the settings store is a GPUI global that only the foreground thread may
//! touch. The writer is the hand-over: [`PrefsWriter::write`] queues a request on a channel and
//! awaits its reply, and one foreground task (owned by the writer, dropped with it) applies each
//! request with [`ClusterSettings::update_cluster`], the comment-preserving editor. Requests are
//! applied in order, one at a time, so two quick toggles never race on the file.

use futures::channel::{mpsc, oneshot};
use futures::future::BoxFuture;
use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, Task};
use oxikube_app::{PrefsPatch, PrefsWriter};
use oxikube_domain::ids::ClusterId;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_settings::ClusterSettings;

struct Request {
    cluster: ClusterId,
    name_hint: Option<String>,
    patch: PrefsPatch,
    reply: oneshot::Sender<OxiResult<()>>,
}

/// Writes posture fields to the user's `settings.json` through the settings store.
///
/// Keep it alive for as long as commands may run (it owns the foreground task that applies the
/// edits); hand `Arc<SettingsPrefsWriter>` to `oxikube_app::register_commands`.
pub struct SettingsPrefsWriter {
    requests: mpsc::UnboundedSender<Request>,
    _worker: Task<()>,
}

impl SettingsPrefsWriter {
    /// A writer applying edits on `cx`'s foreground executor.
    pub fn new(cx: &mut App) -> Self {
        let (requests, mut inbox) = mpsc::unbounded::<Request>();
        let worker = cx.spawn(async move |cx| {
            while let Some(request) = inbox.next().await {
                let Request {
                    cluster,
                    name_hint,
                    patch,
                    reply,
                } = request;
                let edit = cx.update(|cx| {
                    ClusterSettings::update_cluster(cx, &cluster, name_hint.as_deref(), move |c| {
                        if let Some(read_only) = patch.read_only {
                            c.read_only = Some(read_only);
                        }
                        if let Some(colour) = patch.colour {
                            c.colour = colour;
                        }
                    })
                });
                // The caller may have gone away (its dispatch was dropped): the edit still
                // ran, which is what a posture change wants.
                let _ = reply.send(edit.await);
            }
        });
        Self {
            requests,
            _worker: worker,
        }
    }
}

impl PrefsWriter for SettingsPrefsWriter {
    fn write(
        &self,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        patch: PrefsPatch,
    ) -> BoxFuture<'static, OxiResult<()>> {
        let (reply, answer) = oneshot::channel();
        let sent = self.requests.unbounded_send(Request {
            cluster: cluster.clone(),
            name_hint: name_hint.map(str::to_owned),
            patch,
            reply,
        });
        async move {
            sent.map_err(|_| OxiError::internal("the settings writer has shut down"))?;
            answer
                .await
                .map_err(|_| OxiError::internal("the settings writer stopped before answering"))?
        }
        .boxed()
    }
}
