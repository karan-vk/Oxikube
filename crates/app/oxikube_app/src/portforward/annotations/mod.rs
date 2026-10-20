//! k9s-compatible forward annotations (E15-S07): `k9scli.io/port-forwards` and
//! `k9scli.io/auto-port-forwards` on a Pod, read as *suggestions* for the start dialog.
//!
//! Opt-in: `portforward.annotations` ([`ForwardConfig::annotations`], default off). When it is off
//! nothing is parsed and [`annotation_suggestions`] is empty. Nothing ever starts from an
//! annotation: a suggestion only prefills the dialog, and the user still presses Forward. The
//! annotation text is cluster data and therefore untrusted: input is bounded, control characters
//! never reach the display and a bad entry is skipped and reported, never fatal.
//!
//! | Module | What it holds |
//! |---|---|
//! | `parse` | [`parse_k9s_forwards`]: the pure grammar |
//! | `suggest` | [`annotation_suggestions`]: a pod's two annotations, the setting and the conflict check |
//!
//! [`ForwardConfig::annotations`]: super::ForwardConfig::annotations

mod parse;
mod suggest;

#[cfg(test)]
mod tests;

pub use parse::{
    MAX_ANNOTATION_LEN, MAX_ENTRIES, ParseIssue, ParsedForwards, SuggestedForward,
    parse_k9s_forwards,
};
pub use suggest::{
    AUTO_PORT_FORWARDS, AnnotationSuggestion, PORT_FORWARDS, annotation_suggestions,
};
