//! `@events` (E14-S08): events as a block of text for an agent prompt.
//!
//! | Mention | Resolves to |
//! |---|---|
//! | `@events/pod/shop/web-0` | the events about that object, newest first (by last seen) |
//! | `@events/node/worker-1` | the same for a cluster-scoped object |
//! | `@events/shop` | the newest events of the namespace |
//! | `@events` | the newest events of the thread's namespace, else of the cluster |
//!
//! The kind is a word the cluster's alias table knows (`pod`, `deploy`, `Deployment`). The object
//! is read once (a deadline on the app clock) so the block can tell an object that does not exist
//! from one that has no events, and so the events are found through the involved-object index by
//! its uid. An object that is gone but still has events is quoted with a note saying so.
//!
//! Like `k8s.get_events` the provider reads the [`EventService`](crate::events::EventService)
//! (starting the cluster's feed on demand through the same ref-counted subscription), answers an
//! empty buffer with a block that says so, and never fails because the feed is down: the block's
//! notes carry the reason. A mention of the wrong shape is a validation error.
//!
//! Read-only, allowed on a read-only cluster. Every line passes through
//! `oxikube_domain::redact`; the body is cut to the caller's byte budget oldest event first.

use async_trait::async_trait;
use oxikube_domain::agent::ContextBlock;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::redact::redact;
use oxikube_domain::{ErrorKind, OxiResult};
use oxikube_ports::{ContextProviderPort, ContextScope, Mention};
use std::sync::Arc;

use super::block::EventsBlock;
use super::bounds::{MAX_MENTION_EVENTS, READ_DEADLINE};
use super::feed::{OpenFeed, open};
use super::install::EventsAgentDeps;
use super::mention::EventsMention;
use super::query::{Found, Involved, find, find_about, resolve_gvk};
use super::render::one_line;
use crate::events::{EventFilter, ObjectIdentity, ObjectTarget};
use crate::session::ClusterSession;
use crate::tools::metrics::{session_for, within};

/// The mention prefix this provider owns: `@events/...`.
pub const EVENTS_PREFIX: &str = "events";

/// Resolves `@events` mentions.
pub struct EventsContextProvider {
    deps: EventsAgentDeps,
}

impl EventsContextProvider {
    /// A provider reading through `deps`.
    pub fn new(deps: EventsAgentDeps) -> Self {
        Self { deps }
    }
}

impl std::fmt::Debug for EventsContextProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventsContextProvider")
            .finish_non_exhaustive()
    }
}

/// What reading the named object came to.
enum Lookup {
    /// It exists; its uid when the server gave one, and whether it is cluster-scoped.
    Found {
        /// The object's uid, when the server gave one.
        uid: Option<String>,
        /// Whether the object is cluster-scoped (no namespace).
        cluster_scoped: bool,
    },
    /// The cluster has no such object.
    Missing,
    /// It could not be checked (no connection, no permission, too slow); the events are still
    /// quoted by name.
    Unchecked(String),
}

#[async_trait]
impl ContextProviderPort for EventsContextProvider {
    fn mention_prefix(&self) -> &str {
        EVENTS_PREFIX
    }

    async fn resolve(
        &self,
        mention: &Mention,
        scope: &ContextScope,
    ) -> OxiResult<Vec<ContextBlock>> {
        let target = EventsMention::parse(mention.path(), scope.default_namespace.as_deref())?;
        let session = session_for(&self.deps.sessions, scope.cluster.as_ref())?;
        let cluster = session.id().clone();
        let title = session.title().to_owned();
        let header = vec![format!("# cluster: {title} ({cluster})")];
        let block = match target {
            EventsMention::Recent { namespace } => {
                self.recent(&cluster, header, namespace, scope.max_total_bytes)
                    .await
            }
            EventsMention::Object {
                kind,
                namespace,
                name,
            } => {
                self.object(
                    &session,
                    header,
                    (&kind, namespace.as_deref(), &name),
                    scope.max_total_bytes,
                )
                .await
            }
        };
        Ok(vec![block])
    }
}

impl EventsContextProvider {
    /// The newest events of a namespace (or the cluster).
    async fn recent(
        &self,
        cluster: &ClusterId,
        mut header: Vec<String>,
        namespace: Option<String>,
        cap: usize,
    ) -> ContextBlock {
        let mut filter = EventFilter::new();
        let (label, title) = match &namespace {
            Some(ns) => {
                filter = filter.namespaces([ns.as_str()]);
                (
                    format!("namespace {ns}"),
                    format!("Events in namespace {}", one_line(ns, 64)),
                )
            }
            None => (
                "all namespaces".to_owned(),
                "Events in the cluster".to_owned(),
            ),
        };
        header.push(format!("# scope: {}", one_line(&label, 96)));
        let feed = open(&self.deps, cluster).await;
        let found = find(
            &self.deps.events,
            cluster,
            &filter,
            |_| true,
            MAX_MENTION_EVENTS,
        );
        finish(title, header, &feed, &found, Vec::new(), cap)
    }

    /// The events about one object.
    async fn object(
        &self,
        session: &ClusterSession,
        mut header: Vec<String>,
        (kind_word, namespace, name): (&str, Option<&str>, &str),
        cap: usize,
    ) -> ContextBlock {
        let cluster = session.id();
        let gvk = resolve_gvk(&self.deps.aliases, cluster, kind_word);
        let kind = gvk
            .as_ref()
            .map_or_else(|| kind_word.to_owned(), |g| g.kind.to_string());
        let involved = Involved {
            kind,
            namespace: namespace.map(str::to_owned),
            name: name.to_owned(),
        };
        let label = involved.label();
        let title = format!("Events {}", one_line(&label, 160));
        header.push(format!("# object: {}", one_line(&label, 160)));

        let Some(gvk) = gvk else {
            return unknown_kind(&self.deps, cluster, kind_word, title, header);
        };
        let lookup = self.lookup(session, &gvk, namespace, name).await;
        let mut notes = Vec::new();
        let feed = open(&self.deps, cluster).await;
        let mut found = match &lookup {
            // The object is there and namespaced: its events are the ones that carry its uid
            // and the ones that gave no uid and named it.
            Lookup::Found {
                uid: Some(uid),
                cluster_scoped: false,
            } => {
                let target = ObjectTarget::named(
                    Some(Arc::from(uid.as_str())),
                    ObjectIdentity::new(
                        &involved.kind,
                        involved.namespace.as_deref(),
                        &involved.name,
                    ),
                );
                let mut found = find(
                    &self.deps.events,
                    cluster,
                    &EventFilter::new().object_target(target),
                    |_| true,
                    MAX_MENTION_EVENTS,
                );
                if found.total == 0 {
                    // The writer did not set the involved uid: find them by name.
                    found = find_about(&self.deps.events, cluster, &involved, MAX_MENTION_EVENTS);
                }
                found
            }
            // A cluster-scoped object is recorded under its name (the kubelet records a
            // Node's events with the node's name as the uid), and an object that could not
            // be read is matched by name as well.
            _ => find_about(&self.deps.events, cluster, &involved, MAX_MENTION_EVENTS),
        };
        match lookup {
            Lookup::Found { .. } => {}
            Lookup::Missing if found.total == 0 && !feed.unavailable() => {
                return error_block(
                    title,
                    header,
                    &format!(
                        "{label} was not found in this cluster, and no events about it are held"
                    ),
                );
            }
            Lookup::Missing => notes.push(format!(
                "{label} no longer exists in the cluster; the events below were about it"
            )),
            Lookup::Unchecked(why) => notes.push(format!(
                "could not check that {label} exists ({why}); events are matched by name"
            )),
        }
        finish(title, header, &feed, &found, notes, cap)
    }

    /// Reads the object for its uid, within a deadline.
    async fn lookup(
        &self,
        session: &ClusterSession,
        gvk: &Gvk,
        namespace: Option<&str>,
        name: &str,
    ) -> Lookup {
        let Some(reader) = session.resources() else {
            return Lookup::Unchecked("the cluster is not connected".to_owned());
        };
        let read = reader.get_opt(gvk, namespace, name);
        match within(self.deps.clock.as_ref(), READ_DEADLINE, read).await {
            None => Lookup::Unchecked("the read timed out".to_owned()),
            Some(Ok(Some(found))) => Lookup::Found {
                uid: found.meta.uid.as_deref().map(str::to_owned),
                cluster_scoped: found.meta.namespace.is_none(),
            },
            Some(Ok(None)) => Lookup::Missing,
            Some(Err(error)) if error.kind() == ErrorKind::NotFound => Lookup::Missing,
            Some(Err(error)) => Lookup::Unchecked(redact(error.message()).into_owned()),
        }
    }
}

/// The block for what was found, with the feed's own notes first.
fn finish(
    title: String,
    mut header: Vec<String>,
    feed: &OpenFeed,
    found: &Found,
    mut notes: Vec<String>,
    cap: usize,
) -> ContextBlock {
    header.push(format!("# feed: {}", feed.word()));
    header.push(format!(
        "# events: {} held, {} quoted",
        found.total,
        found.events.len()
    ));
    notes.splice(0..0, feed.note());
    EventsBlock {
        title,
        header,
        notes,
        events: &found.events,
        total: found.total,
    }
    .build(cap)
}

/// A block whose body says why there is nothing to quote (`# error: ...`), so the prompt carries
/// the reason instead of a silent gap.
fn error_block(title: String, mut header: Vec<String>, why: &str) -> ContextBlock {
    header.push(format!("# error: {}", one_line(why, 320)));
    ContextBlock::text(title, header.join("\n") + "\n")
}

/// The block for a kind word the cluster does not know, with the closest words.
fn unknown_kind(
    deps: &EventsAgentDeps,
    cluster: &ClusterId,
    word: &str,
    title: String,
    header: Vec<String>,
) -> ContextBlock {
    let mut why = format!("unknown kind {:?} in this cluster", one_line(word, 64));
    if let crate::search::aliases::Resolution::Unknown { suggestions } =
        deps.aliases.table(cluster).resolve(word)
        && !suggestions.is_empty()
    {
        let names: Vec<&str> = suggestions.iter().map(|s| &**s).collect();
        why.push_str(&format!("; did you mean {}?", names.join(", ")));
    }
    error_block(title, header, &why)
}
