//! [`annotation_suggestions`]: a pod's annotations as dialog suggestions.

use oxikube_domain::Resource;

use super::parse::{ParsedForwards, SuggestedForward, parse_k9s_forwards};
use crate::portforward::ForwardInfo;

/// The annotation listing forwards (suggestions).
pub const PORT_FORWARDS: &str = "k9scli.io/port-forwards";

/// The annotation listing forwards k9s starts by itself (suggestions, preselected here).
pub const AUTO_PORT_FORWARDS: &str = "k9scli.io/auto-port-forwards";

/// One suggestion for the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationSuggestion {
    /// What the annotation asks for.
    pub forward: SuggestedForward,
    /// A warning when the fixed local port is taken by a forward that is listed already.
    pub conflict: Option<String>,
}

impl AnnotationSuggestion {
    /// `web 8080:80`: the row's text.
    pub fn label(&self) -> String {
        let f = &self.forward;
        let base = format!("{}:{}", f.local_port, f.remote_port);
        match &f.container {
            Some(container) => format!("{container} {base}"),
            None => base,
        }
    }
}

/// The suggestions on `pod`: its `port-forwards` entries, then its `auto-port-forwards` ones
/// (flagged [`auto`](SuggestedForward::auto); an entry in both is auto), each checked against
/// `active` for a local-port conflict.
///
/// Empty when `enabled` (`portforward.annotations`) is off, when `pod` is not a Pod, or when it
/// has neither annotation; in those cases nothing is parsed. Skipped entries are dropped here
/// (they are logged at debug level, without their text).
pub fn annotation_suggestions(
    pod: &Resource,
    enabled: bool,
    active: &[ForwardInfo],
) -> Vec<AnnotationSuggestion> {
    if !enabled || &*pod.kind.kind != "Pod" {
        return Vec::new();
    }
    let read = |key: &str, auto: bool| -> ParsedForwards {
        pod.meta
            .annotations
            .get(key)
            .map(|value| parse_k9s_forwards(value, auto))
            .unwrap_or_default()
    };
    let (plain, auto) = (read(PORT_FORWARDS, false), read(AUTO_PORT_FORWARDS, true));
    let skipped = plain.issues.len() + auto.issues.len();
    if skipped > 0 {
        tracing::debug!(skipped, "port-forward annotation entries were skipped");
    }
    let mut forwards: Vec<SuggestedForward> = Vec::new();
    for forward in plain.forwards.into_iter().chain(auto.forwards) {
        let twin = forwards.iter_mut().find(|f| {
            (&f.container, f.local_port, f.remote_port)
                == (&forward.container, forward.local_port, forward.remote_port)
        });
        match twin {
            Some(existing) => existing.auto |= forward.auto,
            None => forwards.push(forward),
        }
    }
    forwards
        .into_iter()
        .map(|forward| {
            let conflict = conflict(forward.local_port, active);
            AnnotationSuggestion { forward, conflict }
        })
        .collect()
}

fn conflict(local_port: u16, active: &[ForwardInfo]) -> Option<String> {
    active
        .iter()
        .filter(|info| info.state.is_running())
        .find(|info| info.local_addr.map_or(info.spec.local_port, |a| a.port()) == local_port)
        .map(|info| {
            format!(
                "Local port {local_port} is already used by {}",
                info.label()
            )
        })
}
