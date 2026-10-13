//! `events::SendToAgent` (E14-S08): "Send events to agent" on the command bus.
//!
//! The events view's selection (by uid), a resource's Events tab or table row (an object), or
//! with neither the cluster's newest Warning events, become one [`ContextBlock`] queued in the
//! app's [`PendingContext`](crate::context::PendingContext) with where they came from. The agent
//! panel (E27) drains the queue once it exists; until then the items wait locally, in memory
//! only. The events are read from the [`EventService`](crate::events::EventService) (its feed
//! started on demand), never from the cluster, so the command reads nothing the user's events
//! view does not already hold and changes nothing: no guard tier, allowed on a read-only cluster.
//!
//! The block is the same text `@events` produces ([`EventsBlock`]), secrets masked and cut to
//! the block budget oldest event first. Its MCP stub is `k8s.events_send_to_agent`.

use std::collections::HashSet;
use std::sync::Arc;

use jiff::Timestamp;
use oxikube_domain::command::{self, Command, CommandId};
use oxikube_domain::event::Event;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use serde_json::json;

use super::block::EventsBlock;
use super::bounds::MAX_SENT_EVENTS;
use super::feed::open;
use super::install::EventsAgentDeps;
use super::query::{Found, Involved, find};
use super::render::one_line;
use crate::command_bus::{CommandOutput, CommandRegistry, HandlerContext, RegisterError};
use crate::context::{ContextSource, PendingContext, QueuedContext, Sent};
use crate::events::EventFilter;
use oxikube_domain::agent::MAX_CONTEXT_BLOCK_BYTES;

pub(super) fn register_command(
    registry: &mut CommandRegistry,
    deps: EventsAgentDeps,
    pending: PendingContext,
) -> Result<(), RegisterError> {
    let meta = *command::lookup(CommandId::EVENTS_SEND_TO_AGENT)
        .ok_or(RegisterError::Undeclared(CommandId::EVENTS_SEND_TO_AGENT))?;
    registry.register(meta, move |command: Command, _: HandlerContext| {
        let (deps, pending) = (deps.clone(), pending.clone());
        async move { send(&deps, &pending, command).await }
    })
}

/// What to send, from the payload.
enum Selection {
    /// The events with these uids.
    Uids(HashSet<String>),
    /// Every held event about this object.
    Object(ResourceRef),
    /// The cluster's newest Warning events.
    Warnings,
}

async fn send(
    deps: &EventsAgentDeps,
    pending: &PendingContext,
    command: Command,
) -> OxiResult<CommandOutput> {
    let Command::EventsSendToAgent {
        cluster,
        target,
        events,
    } = command
    else {
        return Err(OxiError::validation("not an events::SendToAgent command"));
    };
    if let Some(target) = &target
        && target.cluster != cluster
    {
        return Err(OxiError::validation(
            "events::SendToAgent: the target object is in another cluster",
        ));
    }
    let session = deps
        .sessions
        .get(&cluster)
        .ok_or_else(|| OxiError::not_found(format!("no session for cluster {cluster}")))?;
    let title = session.title().to_owned();
    let selection = if !events.is_empty() {
        Selection::Uids(events.into_iter().collect())
    } else if let Some(target) = target {
        Selection::Object(target)
    } else {
        Selection::Warnings
    };

    let feed = open(deps, &cluster).await;
    if feed.unavailable() {
        return Err(OxiError::network(
            feed.note()
                .unwrap_or_else(|| "events are unavailable".to_owned()),
        ));
    }
    let (found, subject, namespace) = read(deps, &cluster, &selection);
    if found.events.is_empty() {
        let message = match selection {
            Selection::Uids(_) => "None of the selected events is held any more.",
            Selection::Object(_) => "There are no events about this object to send.",
            Selection::Warnings => "There are no warning events to send.",
        };
        return Ok(CommandOutput {
            message: Some(message.to_owned()),
            data: Some(json!({ "sent": 0 })),
        });
    }

    let mut header = vec![
        format!("# cluster: {title} ({cluster})"),
        format!("# subject: {}", one_line(&subject, 160)),
        format!(
            "# events: {} held, {} sent",
            found.total,
            found.events.len()
        ),
    ];
    if let Some(ns) = &namespace {
        header.push(format!("# namespace: {}", one_line(ns, 64)));
    }
    let block = EventsBlock {
        title: format!("Events: {}", one_line(&subject, 160)),
        header,
        notes: feed.note().into_iter().collect(),
        events: &found.events,
        total: found.total,
    }
    .build(MAX_CONTEXT_BLOCK_BYTES);
    let count = found.events.len();
    let item = QueuedContext {
        block,
        source: ContextSource {
            cluster: cluster.clone(),
            cluster_name: title,
            namespace: namespace.unwrap_or_default(),
            subject,
            container: None,
            span: span(&found.events),
            lines: count,
        },
    };
    let subject = item.source.subject.clone();
    let noun = if count == 1 { "event" } else { "events" };
    let (message, delivery) = match pending.send(item) {
        Sent::Delivered => (format!("Sent {count} {noun} to the agent"), "delivered"),
        Sent::Queued(waiting) => (
            format!(
                "Queued {count} {noun} for the agent ({waiting} waiting). They are delivered \
                 when the agent panel opens."
            ),
            "queued",
        ),
    };
    Ok(CommandOutput {
        message: Some(message),
        data: Some(json!({
            "sent": count,
            "total": found.total,
            "delivery": delivery,
            "subject": subject,
        })),
    })
}

/// The events `selection` names, what to call them, and their namespace (for the source).
fn read(
    deps: &EventsAgentDeps,
    cluster: &oxikube_domain::ids::ClusterId,
    selection: &Selection,
) -> (Found, String, Option<String>) {
    match selection {
        Selection::Uids(uids) => {
            let found = deps
                .events
                .read(cluster, |buffer| {
                    let mut found = Found::default();
                    for event in buffer.query(&EventFilter::new()) {
                        if event.uid.as_deref().is_some_and(|uid| uids.contains(uid)) {
                            found.total += 1;
                            if found.events.len() < MAX_SENT_EVENTS {
                                found.events.push(event.clone());
                            }
                            if found.total == uids.len() {
                                break;
                            }
                        }
                    }
                    found
                })
                .unwrap_or_default();
            let noun = if found.total == 1 { "event" } else { "events" };
            let subject = format!("{} selected {noun}", found.total);
            (found, subject, None)
        }
        Selection::Object(object) => {
            let involved = Involved::of(object);
            let filter = EventFilter::new().kinds([involved.kind.as_str()]);
            let found = find(
                &deps.events,
                cluster,
                &filter,
                Some(&involved),
                MAX_SENT_EVENTS,
            );
            let subject = format!("events of {}", involved.label());
            (found, subject, involved.namespace)
        }
        Selection::Warnings => {
            let found = find(
                &deps.events,
                cluster,
                &EventFilter::warnings(),
                None,
                MAX_SENT_EVENTS,
            );
            (found, "the newest warning events".to_owned(), None)
        }
    }
}

/// The oldest and the newest time the `events` were last seen.
fn span(events: &[Arc<Event>]) -> Option<(Timestamp, Timestamp)> {
    let mut times = events.iter().filter_map(|e| e.last_seen.or(e.first_seen));
    let first = times.next()?;
    Some(times.fold((first, first), |(lo, hi), t| (lo.min(t), hi.max(t))))
}
