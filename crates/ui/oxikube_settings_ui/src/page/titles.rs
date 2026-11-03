//! Human titles for setting keys, and the page's optional per-section metadata.
//!
//! Titles are derived from the key (`font_size` is "Font size"); the table below only adds what
//! a key cannot say: a friendlier section title and a one-line blurb in place of the content
//! struct's doc comment ("What one settings layer says about ..."), which is written for the
//! schema, not for the page.

/// Words shown in capitals inside a title.
const ACRONYMS: &[&str] = &[
    "api", "cpu", "cwd", "gpu", "id", "ip", "json", "os", "tls", "ttl", "ui", "url", "uri",
];

/// `(key, title, blurb)` of the sections that read better with their own wording.
const SECTION_META: &[(&str, &str, &str)] = &[
    (
        "audit",
        "Audit log",
        "How long the local audit log keeps its records.",
    ),
    (
        "cloud",
        "Cloud discovery",
        "Which cloud providers are scanned for clusters, and with which tools.",
    ),
    ("describe", "Describe", "How `describe` output is produced."),
    (
        "diff",
        "Resource diff",
        "Paths the cross-cluster resource diff ignores.",
    ),
    (
        "editor",
        "Manifest editor",
        "Fonts, wrapping, validation and the field manager of the manifest editor.",
    ),
    (
        "events",
        "Events and notifications",
        "Which events are watched and when they notify.",
    ),
    ("helm", "Helm", "The `helm` binary and chart repositories."),
    (
        "kubeconfig",
        "Kubeconfig",
        "The kubeconfig files and folders the catalog reads.",
    ),
    ("kustomize", "Kustomize", "The binary that builds overlays."),
    (
        "log",
        "Diagnostics log",
        "What Oxikube writes to its own log.",
    ),
    (
        "logs",
        "Log viewer",
        "Defaults of the pod log viewer: range, wrapping, timestamps.",
    ),
    (
        "metrics",
        "Metrics",
        "Live usage and history graphs of the cluster's resources.",
    ),
    (
        "portforward",
        "Port forwarding",
        "The local address, ports and read-only policy of port-forwards.",
    ),
    (
        "resource_table",
        "Resource tables",
        "How the resource tables list and refresh objects.",
    ),
    (
        "session",
        "Session restore",
        "What is reopened when Oxikube starts again.",
    ),
    (
        "terminal",
        "Terminal",
        "Shell, font, cursor and scrollback of terminals.",
    ),
];

/// The key of the section that holds the top-level fields that are not objects.
pub const GENERAL_KEY: &str = "general";

/// The title and blurb the page gives the section `key`, when it has its own.
pub(super) fn section_meta(key: &str) -> Option<(&'static str, &'static str)> {
    SECTION_META
        .iter()
        .find(|(candidate, _, _)| *candidate == key)
        .map(|(_, title, blurb)| (*title, *blurb))
}

/// `font_size` as "Font size", `ui_scale` as "UI scale": words split on `_`, the first one
/// capitalised, known acronyms in capitals.
pub fn humanize(key: &str) -> String {
    let mut title = String::with_capacity(key.len());
    for (ix, word) in key.split('_').filter(|word| !word.is_empty()).enumerate() {
        if ix > 0 {
            title.push(' ');
        }
        if ACRONYMS.contains(&word) {
            title.push_str(&word.to_ascii_uppercase());
        } else if ix == 0 {
            let mut chars = word.chars();
            if let Some(first) = chars.next() {
                title.extend(first.to_uppercase());
                title.push_str(chars.as_str());
            }
        } else {
            title.push_str(word);
        }
    }
    title
}
