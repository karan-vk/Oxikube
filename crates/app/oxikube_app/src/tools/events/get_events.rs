//! `k8s.get_events`: the events the user reads in the events view, for an agent.

use std::fmt::Write as _;

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::redact::redact;
use oxikube_ports::{
    ContentPart, ToolAnnotations, ToolContext, ToolDef, ToolName, ToolOutput, ToolPort,
};
use serde_json::{Value, json};

use super::args::{GetEventsArgs, InvolvedArg};
use super::bounds::{DEFAULT_LIMIT, MAX_LIMIT, MAX_MESSAGE_CHARS};
use super::feed::open;
use super::install::EventsAgentDeps;
use super::query::{Found, Involved, find, kind_name};
use super::render::{TSV_HEADER, json_row, one_line, tsv_line};
use crate::events::EventFilter;
use crate::tools::metrics::session_for;

/// The tool's name.
pub const GET_EVENTS: &str = "k8s.get_events";

/// The `k8s.get_events` tool: see the [module docs](super).
pub struct GetEventsTool {
    def: ToolDef,
    deps: EventsAgentDeps,
}

impl GetEventsTool {
    /// The tool over `deps`.
    pub fn new(deps: EventsAgentDeps) -> Self {
        Self {
            def: definition(),
            deps,
        }
    }
}

impl std::fmt::Debug for GetEventsTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetEventsTool").finish_non_exhaustive()
    }
}

#[async_trait]
impl ToolPort for GetEventsTool {
    fn def(&self) -> &ToolDef {
        &self.def
    }

    async fn invoke(&self, args: Value, ctx: &ToolContext) -> OxiResult<ToolOutput> {
        let now = self.deps.clock.now();
        let call = GetEventsArgs::parse(&args, now)?;
        let session = match session_for(&self.deps.sessions, ctx.cluster.as_ref()) {
            Ok(session) => session,
            Err(error) if error.kind() == oxikube_domain::ErrorKind::Validation => {
                return Err(error);
            }
            Err(error) => {
                return Ok(ToolOutput::error(format!(
                    "{}: {}",
                    error.kind(),
                    redact(error.message())
                )));
            }
        };
        let (cluster, title) = (session.id().clone(), session.title().to_owned());

        let (filter, object) = build_filter(&self.deps, &cluster, &call);
        let feed = open(&self.deps, &cluster).await;
        let found = find(
            &self.deps.events,
            &cluster,
            &filter,
            object.as_ref(),
            call.limit,
        );

        let mut notes: Vec<String> = feed.note().into_iter().collect();
        if feed.unavailable() {
            return Ok(unavailable(&title, notes.pop().unwrap_or_default()));
        }
        if found.total == 0 {
            notes.push(
                "no events match: the cluster keeps events for about an hour, and only those \
                 the app has watched since the feed started are held"
                    .to_owned(),
            );
        } else if found.truncated() {
            notes.push(format!(
                "{} of {} matching events are listed; narrow the query with `ns`, `involved`, \
                 `since` or `type`, or lower `limit`",
                found.events.len(),
                found.total
            ));
        }
        Ok(render(&Report {
            cluster: &title,
            call: &call,
            feed: feed.word(),
            found: &found,
            notes: &notes,
            now,
        }))
    }
}

/// The filter and the post-filter an `involved` object needs.
fn build_filter(
    deps: &EventsAgentDeps,
    cluster: &oxikube_domain::ids::ClusterId,
    call: &GetEventsArgs,
) -> (EventFilter, Option<Involved>) {
    let mut filter = EventFilter::new();
    if let Some(ns) = &call.namespace {
        filter = filter.namespaces([ns.as_str()]);
    }
    if let Some(since) = call.since {
        filter = filter.since(since);
    }
    if let Some(kind) = call.event_type {
        filter = filter.types([kind]);
    }
    let mut object = None;
    match &call.involved {
        Some(InvolvedArg::Uid(uid)) => filter = filter.object(uid.as_str()),
        Some(InvolvedArg::Object { kind, name }) => {
            let kind = kind_name(&deps.aliases, cluster, kind);
            filter = filter.kinds([kind.as_str()]);
            object = Some(Involved {
                kind,
                namespace: None,
                name: name.clone(),
            });
        }
        None => {}
    }
    (filter, object)
}

/// Everything the answer says.
struct Report<'a> {
    cluster: &'a str,
    call: &'a GetEventsArgs,
    feed: &'static str,
    found: &'a Found,
    notes: &'a [String],
    now: jiff::Timestamp,
}

fn scope_text(call: &GetEventsArgs) -> String {
    let mut scope = call.namespace.as_deref().map_or_else(
        || "all namespaces".to_owned(),
        |ns| format!("namespace {ns}"),
    );
    match &call.involved {
        Some(InvolvedArg::Uid(uid)) => {
            let _ = write!(scope, ", object uid {}", one_line(uid, 64));
        }
        Some(InvolvedArg::Object { kind, name }) => {
            let _ = write!(scope, ", {} {}", one_line(kind, 64), one_line(name, 128));
        }
        None => {}
    }
    if let Some(since) = call.since {
        let _ = write!(scope, ", since {since}");
    }
    scope
}

fn render(report: &Report<'_>) -> ToolOutput {
    let Report {
        cluster,
        call,
        feed,
        found,
        notes,
        now,
    } = report;
    let mut text = format!(
        "{} of {} events in {} on {cluster}, newest first{}\n",
        found.events.len(),
        found.total,
        scope_text(call),
        if found.truncated() {
            " (truncated)"
        } else {
            ""
        },
    );
    if !found.events.is_empty() {
        text.push_str(TSV_HEADER);
        text.push('\n');
    }
    for event in &found.events {
        text.push_str(&tsv_line(event, *now, MAX_MESSAGE_CHARS));
        text.push('\n');
    }
    for note in *notes {
        let _ = writeln!(text, "note: {note}");
    }
    let rows: Vec<Value> = found
        .events
        .iter()
        .map(|event| json_row(event, *now, MAX_MESSAGE_CHARS))
        .collect();
    ToolOutput {
        content: vec![ContentPart::text(text)],
        structured: Some(json!({
            "status": "ok",
            "cluster": cluster,
            "namespace": call.namespace,
            "since": call.since.map(|t| t.to_string()),
            "feed": feed,
            "total": found.total,
            "returned": found.events.len(),
            "truncated": found.truncated(),
            "note": (!notes.is_empty()).then(|| notes.join("; ")),
            "events": rows,
        })),
        is_error: false,
    }
}

/// `events unavailable: <reason>` as a normal result, like the metrics tools' absence: not an
/// error, and not an empty list (the model can tell "no events" from "cannot say").
fn unavailable(cluster: &str, reason: String) -> ToolOutput {
    ToolOutput {
        content: vec![ContentPart::text(format!(
            "events unavailable on {cluster}: {reason}"
        ))],
        structured: Some(json!({
            "status": "unavailable",
            "cluster": cluster,
            "reason": reason,
            "total": 0,
            "returned": 0,
            "truncated": false,
            "events": [],
        })),
        is_error: false,
    }
}

fn definition() -> ToolDef {
    let name = ToolName::new(GET_EVENTS).expect("a well-formed tool name");
    ToolDef::read_only(
        name,
        "List Kubernetes events from the events Oxikube holds for the cluster (like `kubectl get \
         events`, newest first): type, reason, object, count, age and message. Filter by \
         namespace (`ns`), by the object they are about (`involved`: a kind and name, or a uid), \
         by time (`since`: a duration like 15m or an RFC 3339 timestamp) and by `type`. The list \
         is capped (`limit`, at most 100): `truncated` and `total` say when to narrow the \
         query. No events is an empty list with a note; a cluster whose events cannot be read has \
         status \"unavailable\" and the reason.",
        json!({
            "type": "object",
            "properties": {
                "ns": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Only events stored in this namespace (as kubectl -n). \
                                    Default or \"all\": every namespace. Leave it out for \
                                    cluster-scoped objects such as nodes."
                },
                "involved": {
                    "type": "object",
                    "description": "Only events about this object: its `kind` (Pod, \
                                    Deployment, or a short name like deploy) and `name`, or \
                                    its `uid`.",
                    "properties": {
                        "kind": { "type": "string", "minLength": 1 },
                        "name": { "type": "string", "minLength": 1 },
                        "uid": { "type": "string", "minLength": 1 }
                    },
                    "additionalProperties": false
                },
                "since": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Only events last seen since then: a duration back from now \
                                    (90s, 15m, 2h, 1d; up to 7d) or an RFC 3339 timestamp. \
                                    Default: all that are held."
                },
                "type": {
                    "type": "string",
                    "enum": ["Warning", "Normal"],
                    "description": "Only events of this type. Default: both."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_LIMIT,
                    "description": format!("How many events to return. Default {DEFAULT_LIMIT}.")
                }
            },
            "additionalProperties": false
        }),
    )
    .with_title("Get events")
    .with_annotations(ToolAnnotations {
        idempotent: true,
        ..ToolAnnotations::default()
    })
}
