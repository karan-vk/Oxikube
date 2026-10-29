//! The dry-run preview (E19-S07): what the server says a write would do, attached to the
//! confirmation the guard asks for.
//!
//! The guard previews every command whose [`MutationVerb`] is a create, update, patch or apply
//! ([`preview_verb`]), whatever its tier, and every command whose risk
//! [requires a dry run](oxikube_domain::safety::Risk::requires_dry_run_diff). The preview is the
//! command's own handler run once against a writer that forces `dryRun=All` on every request
//! (`ReadOnlyGate::rehearsal`) and keeps what the server answered ([`WriteLog`]). Then:
//!
//! * each object written is read live and compared with the server's result
//!   ([`PreviewChange`]): the diff the dialog draws with the E10 diff view. Both sides of a Secret
//!   have their values replaced by keyed fingerprints first (the apply module's mask), so a
//!   preview never carries a Secret value;
//! * the API server's `Warning:` headers that arrived while the dry run ran are kept
//!   ([`DryRunResult::warnings`]);
//! * a server that cannot dry-run the request (405, 415 or 501, an aggregated API that answers
//!   "dry run is not supported", an admission webhook with side effects refusing a dry run) gives
//!   [`NoPreviewAvailable`](super::DryRunPreview::NoPreviewAvailable) with its words
//!   ([`no_preview_reason`]); that never blocks the mutation.
//!
//! A delete writes no object, so it carries no diff: its preview is the handler's message.
//! The confirmed dispatch runs the real request; the dry run's result is never reused as the
//! commit.

use std::sync::Arc;

use futures::FutureExt as _;
use futures::StreamExt as _;
use futures::stream::BoxStream;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::{ClusterId, Gvk, ResourceRef};
use oxikube_domain::safety::MutationVerb;
use oxikube_domain::{ErrorKind, OxiError, Resource};
use oxikube_ports::{ApiWarning, ResourceReader};
use parking_lot::Mutex;
use serde_json::Value;

use crate::apply::{Change, SecretMask};

/// Most objects one preview compares; the rest are counted in [`DryRunResult::omitted`].
pub const MAX_PREVIEW_OBJECTS: usize = 8;

/// Most server warnings one preview keeps.
const MAX_PREVIEW_WARNINGS: usize = 16;

/// The commands the guard dry-runs before asking, by what they do to their objects. A command
/// that writes through a port other than the `ResourceWriter` (Helm), returns a credential
/// (`serviceaccount::GenerateKubeconfig`), opens a session (`pod::Debug`) or acts on many
/// objects at once (`selection::Label`) is not here: its dry run would not be a preview.
const PREVIEWED: &[(CommandId, MutationVerb)] = &[
    (CommandId::RESOURCE_APPLY, MutationVerb::Apply),
    (CommandId::NAMESPACE_CREATE, MutationVerb::Create),
    (CommandId::ROLE_CREATE, MutationVerb::Create),
    (CommandId::BINDING_CREATE, MutationVerb::Create),
    (CommandId::CRONJOB_TRIGGER, MutationVerb::Create),
    (CommandId::ROLE_EDIT, MutationVerb::Update),
    (CommandId::BINDING_EDIT, MutationVerb::Update),
    (CommandId::CRONJOB_SUSPEND, MutationVerb::Patch),
    (CommandId::CRONJOB_RESUME, MutationVerb::Patch),
    (CommandId::JOB_SUSPEND, MutationVerb::Patch),
    (CommandId::JOB_RESUME, MutationVerb::Patch),
    (CommandId::WORKLOAD_SET_IMAGE, MutationVerb::Patch),
    (CommandId::WORKLOAD_ROLLBACK, MutationVerb::Patch),
    (CommandId::PVC_RESIZE, MutationVerb::Patch),
    (CommandId::INGRESS_CLASS_SET_DEFAULT, MutationVerb::Patch),
    (CommandId::STORAGECLASS_SET_DEFAULT, MutationVerb::Patch),
];

/// What `command` does when the guard previews it: a create, update, patch or apply; `None`
/// for a command the guard does not preview (it may still dry-run it for its risk).
pub fn preview_verb(command: &Command) -> Option<MutationVerb> {
    let id = command.id();
    PREVIEWED
        .iter()
        .find_map(|(previewed, verb)| (*previewed == id).then_some(*verb))
}

/// What the server-side dry run of a command showed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DryRunResult {
    /// What the handler reported (`Dry run: namespace demo can be deleted`).
    pub message: Option<String>,
    /// Each object the dry run wrote: the live object next to the server's result, in the order
    /// they were written, at most [`MAX_PREVIEW_OBJECTS`].
    pub changes: Vec<PreviewChange>,
    /// How many more objects the dry run wrote than [`changes`](Self::changes) holds.
    pub omitted: usize,
    /// The API server's `Warning:` headers that arrived while the dry run ran (deprecations,
    /// unknown fields, admission notes), redacted, each once.
    pub warnings: Vec<String>,
}

/// One object of a preview: the live object and what the server would store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewChange {
    /// The object.
    pub target: ResourceRef,
    /// Created, changed, or left as it is.
    pub change: Change,
    /// The live object (`None` for a create, or when it could not be read); a Secret's values
    /// are fingerprints.
    pub live: Option<Arc<Value>>,
    /// The object as the server would store it; a Secret's values are fingerprints.
    pub result: Arc<Value>,
}

/// One write a rehearsal sent and the server's answer.
struct Written {
    target: ResourceRef,
    created: bool,
    object: Resource,
}

/// The objects a rehearsal's writer got back from the server, capped at [`MAX_PREVIEW_OBJECTS`]
/// distinct objects (a later write of the same object replaces the earlier answer).
#[derive(Default)]
pub(super) struct WriteLog {
    written: Mutex<(Vec<Written>, usize)>,
}

impl WriteLog {
    /// Records the server's `object` for a dry-run write of `kind` on `cluster`; `created` for a
    /// create.
    pub(super) fn record(&self, cluster: &ClusterId, kind: &Gvk, created: bool, object: &Resource) {
        let target = ResourceRef::new(
            cluster.clone(),
            kind.clone(),
            object.meta.namespace.clone(),
            object.meta.name.clone(),
        );
        let mut guard = self.written.lock();
        let (written, omitted) = &mut *guard;
        if let Some(earlier) = written.iter_mut().find(|w| w.target == target) {
            earlier.object = object.clone();
            earlier.created |= created;
        } else if written.len() < MAX_PREVIEW_OBJECTS {
            written.push(Written {
                target,
                created,
                object: object.clone(),
            });
        } else {
            *omitted += 1;
        }
    }

    fn take(&self) -> (Vec<Written>, usize) {
        std::mem::take(&mut *self.written.lock())
    }
}

/// The preview of a dry run that succeeded: `message` from the handler, each object in `log`
/// next to its live version read through `reader`, and the `warnings` that arrived meanwhile.
pub(super) async fn collect(
    message: Option<String>,
    log: &WriteLog,
    reader: Option<Arc<dyn ResourceReader>>,
    warnings: Vec<String>,
) -> DryRunResult {
    let (written, omitted) = log.take();
    let mask = SecretMask::new();
    let mut changes = Vec::with_capacity(written.len());
    for Written {
        target,
        created,
        object,
    } in written
    {
        // `Err` when the live object could not be read (no reader, an error): then the result
        // shows as a change against nothing.
        let live = match (&reader, created) {
            (_, true) => Ok(None),
            (Some(reader), false) => reader
                .get_opt(&target.gvk, target.namespace(), &target.name)
                .await
                .map(|live| live.map(|live| mask.apply(live.to_value()))),
            (None, false) => Err(OxiError::conflict("the cluster is not connected")),
        };
        let result = mask.apply(object.to_value());
        let (change, live) = match live {
            // A patch or apply of an object that does not exist yet (server-side apply) creates it.
            Ok(None) => (Change::Create, None),
            Ok(Some(live)) => (Change::between(Some(&live), &result), Some(live)),
            Err(_) => (Change::Update, None),
        };
        changes.push(PreviewChange {
            target,
            change,
            live: live.map(Arc::new),
            result: Arc::new(result),
        });
    }
    DryRunResult {
        message,
        changes,
        omitted,
        warnings,
    }
}

/// The warnings `stream` holds now, each once, at most [`MAX_PREVIEW_WARNINGS`]. Never waits.
pub(super) fn drain_warnings(stream: Option<BoxStream<'static, ApiWarning>>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(mut stream) = stream else {
        return out;
    };
    while let Some(Some(warning)) = stream.next().now_or_never() {
        if out.len() < MAX_PREVIEW_WARNINGS && !out.contains(&warning.text) {
            out.push(warning.text);
        }
    }
    out
}

/// Why the server could not dry-run a request, when `error` says it cannot (rather than that the
/// request is wrong): an `Unsupported` error (405, 406, 415, 501, a missing API), or any error
/// whose message says dry run is not supported (an aggregated API's 400, an admission webhook
/// with side effects refusing a dry-run request). `None` for any other error: the real request
/// would fail the same way.
pub fn no_preview_reason(error: &OxiError) -> Option<String> {
    if error.kind() == ErrorKind::Unsupported || says_dry_run_unsupported(error.message()) {
        Some(error.message().to_owned())
    } else {
        None
    }
}

fn says_dry_run_unsupported(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    let names_dry_run = ["dry run", "dryrun", "dry-run"]
        .iter()
        .any(|word| message.contains(word));
    names_dry_run && message.contains("not support")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_writes_of_an_object_and_nothing_else() {
        let scale = Command::WorkloadScale {
            target: ResourceRef::namespaced(
                ClusterId::new("/k", &oxikube_domain::ids::ContextName::new("a")),
                Gvk::new("apps", "v1", "Deployment"),
                "shop",
                "web",
            ),
            replicas: 2,
        };
        assert_eq!(
            preview_verb(&scale),
            None,
            "a scale is not a create/update/patch/apply"
        );
        for (id, verb) in PREVIEWED {
            assert!(
                matches!(
                    verb,
                    MutationVerb::Create
                        | MutationVerb::Update
                        | MutationVerb::Patch
                        | MutationVerb::Apply
                ),
                "{id} previews as {verb}"
            );
            let meta = oxikube_domain::command::lookup(*id).expect("declared");
            assert!(meta.mutating, "{id} is a mutation");
            assert!(!meta.exec, "{id} opens no session");
        }
    }

    #[test]
    fn unsupported_dry_runs_are_told_apart_from_refusals() {
        let unsupported = OxiError::new(ErrorKind::Unsupported, "the server does not allow PATCH");
        assert_eq!(
            no_preview_reason(&unsupported).as_deref(),
            Some("the server does not allow PATCH")
        );
        for text in [
            "admission webhook \"audit.example.com\" does not support dry run",
            "dryRun is not supported by this API",
            "the aggregated API does not support dry-run requests",
        ] {
            let error = OxiError::validation(text);
            assert_eq!(no_preview_reason(&error).as_deref(), Some(text), "{text}");
        }
        for error in [
            OxiError::validation("spec.replicas: Invalid value: -1"),
            OxiError::new(ErrorKind::Forbidden, "deployments is forbidden"),
            OxiError::conflict("the object has been modified"),
        ] {
            assert_eq!(no_preview_reason(&error), None, "{error}");
        }
    }
}
