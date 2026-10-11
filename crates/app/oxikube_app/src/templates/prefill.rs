//! [`Prefill`]: a template's text with the namespace of the selection in it.
//!
//! A template keeps `namespace: default` where the namespace goes, so it is valid YAML (and
//! validates clean) before anything is substituted. Every line whose key is `namespace` and whose
//! value is `default` is a placeholder; a template that says another namespace, or none (a
//! Namespace, a ClusterRole), is left alone.

use oxikube_domain::session::NamespaceSelection;

/// The namespace the placeholders hold unless the selection names one.
const PLACEHOLDER: &str = "default";

/// What a new editor tab opens with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prefill {
    /// The template's text, placeholders replaced.
    pub text: String,
    /// The namespace put in the placeholders; `None` when the template has none (a cluster-scoped
    /// kind).
    pub namespace: Option<String>,
    /// What to tell the user when the selection did not name one namespace ("All namespaces are
    /// selected: the namespace is set to default").
    pub note: Option<String>,
}

/// Replaces the placeholders of `body` with the namespace `selection` names: the only one, else the
/// first of several, else `default` for all namespaces; the note says so in the last two cases.
pub(super) fn prefill(body: &str, selection: &NamespaceSelection) -> Prefill {
    let names: Vec<&str> = selection.names().collect();
    let (namespace, reason) = match names.as_slice() {
        [] => (PLACEHOLDER, Some("All namespaces are selected".to_owned())),
        [only] => (*only, None),
        [first, ..] => (
            *first,
            Some(format!("{} namespaces are selected", names.len())),
        ),
    };
    let mut text = String::with_capacity(body.len());
    let mut replaced = false;
    for line in body.split_inclusive('\n') {
        match substitute(line, namespace) {
            Some(line) => {
                replaced = true;
                text.push_str(&line);
            }
            None => text.push_str(line),
        }
    }
    if !replaced {
        return Prefill {
            text,
            namespace: None,
            note: None,
        };
    }
    // The note only matters when the chosen namespace is not the one the user picked.
    let note = reason.map(|reason| {
        if namespace == PLACEHOLDER {
            format!("{reason}: the namespace is set to {PLACEHOLDER}")
        } else {
            format!("{reason}: the namespace is set to {namespace}, the first")
        }
    });
    Prefill {
        text,
        namespace: Some(namespace.to_owned()),
        note,
    }
}

/// `line` with its `namespace: default` value replaced by `namespace`; `None` for any other line.
fn substitute(line: &str, namespace: &str) -> Option<String> {
    let body = line.trim_end_matches(['\n', '\r']);
    let ending = &line[body.len()..];
    let indent_len = body.len() - body.trim_start().len();
    let rest = body[indent_len..].strip_prefix("namespace:")?;
    // `namespace: default  # note`: the value ends at a comment, whose spacing is kept.
    let value_end = rest.find(" #").unwrap_or(rest.len());
    let value = rest[..value_end].trim();
    let tail = &rest[rest[..value_end].trim_end().len()..];
    let value = value.trim_matches(['"', '\'']);
    if value != PLACEHOLDER {
        return None;
    }
    Some(format!(
        "{}namespace: {namespace}{tail}{ending}",
        &body[..indent_len]
    ))
}
