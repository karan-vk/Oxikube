//! [`EventsFilters`]: what the events view's toolbar says to show, as the user typed it, and the
//! [`EventFilter`] it compiles to.
//!
//! The text fields hold what was typed; [`EventsFilters::compile`] turns them into the service's
//! filter once per edit (the regex is compiled there, never per event). Reason, kind and
//! namespace take a list (`BackOff, Failed`): any of them matches. The text is a
//! case-insensitive substring, or a regular expression when it has regex syntax; one that does
//! not compile is an [`EventFilterError`] the toolbar shows while the view keeps the rows of the
//! last good filter. The whole struct is what is saved per cluster (no secrets: only what the
//! user typed).

use oxikube_app::events::{EventFilter, EventFilterError};
use oxikube_domain::event::EventType;
use serde::{Deserialize, Serialize};

/// The type part of the toolbar: every event, or one type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TypeChoice {
    /// Every type.
    #[default]
    All,
    /// `Normal` events only.
    Normal,
    /// `Warning` events only.
    Warning,
}

impl TypeChoice {
    /// The three choices, in toolbar order.
    pub const ALL: [TypeChoice; 3] = [TypeChoice::All, TypeChoice::Normal, TypeChoice::Warning];

    /// The toolbar label.
    pub const fn label(self) -> &'static str {
        match self {
            TypeChoice::All => "All",
            TypeChoice::Normal => "Normal",
            TypeChoice::Warning => "Warning",
        }
    }

    fn event_type(self) -> Option<EventType> {
        match self {
            TypeChoice::All => None,
            TypeChoice::Normal => Some(EventType::Normal),
            TypeChoice::Warning => Some(EventType::Warning),
        }
    }
}

/// What the toolbar asks for, as typed. See the [module docs](self).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventsFilters {
    /// Only the faults (`Warning` events): k9s' `ctrl-z`. Overrides [`types`](Self::types).
    pub faults_only: bool,
    /// The type choice.
    pub types: TypeChoice,
    /// Reasons, comma or space separated (`BackOff, Failed`); compared exactly.
    pub reason: String,
    /// Kinds of the involved object (`Pod, Node`); case-insensitive.
    pub kind: String,
    /// Namespaces the events are stored in (`kube-system`).
    pub namespace: String,
    /// The text: a substring, or a regular expression.
    pub text: String,
}

impl EventsFilters {
    /// The service filter these say.
    ///
    /// # Errors
    ///
    /// [`EventFilterError`] when the text is a regular expression that does not compile.
    pub fn compile(&self) -> Result<EventFilter, EventFilterError> {
        let types = if self.faults_only {
            Some(EventType::Warning)
        } else {
            self.types.event_type()
        };
        EventFilter::new()
            .types(types)
            .reasons(words(&self.reason))
            .kinds(words(&self.kind))
            .namespaces(words(&self.namespace))
            .text(self.text.trim())
    }

    /// Whether anything narrows the events (the faults toggle included).
    pub fn is_active(&self) -> bool {
        *self != Self::default()
    }
}

/// The words of a list field: split on commas and white space, empty ones dropped.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|word| !word.is_empty())
}

#[cfg(test)]
mod tests {
    use oxikube_testkit::event;

    use super::*;

    fn filters() -> EventsFilters {
        EventsFilters::default()
    }

    #[test]
    fn faults_only_overrides_the_type_choice() {
        let warning = event().warning().reason("BackOff").build();
        let normal = event().reason("Pulled").build();
        let only_normal = EventsFilters {
            types: TypeChoice::Normal,
            ..filters()
        };
        let compiled = only_normal.compile().unwrap();
        assert!(compiled.matches(&normal) && !compiled.matches(&warning));
        let faults = EventsFilters {
            faults_only: true,
            ..only_normal
        }
        .compile()
        .unwrap();
        assert!(faults.is_warnings_only());
        assert!(faults.matches(&warning) && !faults.matches(&normal));
    }

    #[test]
    fn list_fields_take_any_of_several_words() {
        let compiled = EventsFilters {
            reason: "BackOff, Failed  Killing".into(),
            kind: "pod".into(),
            ..filters()
        }
        .compile()
        .unwrap();
        let about_pod = |reason: &str| {
            event()
                .reason(reason)
                .regarding("Pod", Some("default"), "web", "uid")
                .build()
        };
        assert!(compiled.matches(&about_pod("BackOff")));
        assert!(compiled.matches(&about_pod("Killing")));
        assert!(!compiled.matches(&about_pod("Pulled")));
        let about_node = event()
            .reason("Failed")
            .regarding("Node", None, "n1", "n1")
            .build();
        assert!(!compiled.matches(&about_node));
    }

    #[test]
    fn a_bad_regex_is_an_error_and_an_empty_one_matches_everything() {
        let bad = EventsFilters {
            text: "back(off".into(),
            ..filters()
        };
        let error = bad.compile().expect_err("unbalanced");
        assert_eq!(error.pattern, "back(off");
        assert!(filters().compile().unwrap().is_empty());
        assert!(!filters().is_active());
        assert!(bad.is_active());
    }

    #[test]
    fn the_saved_form_reads_back_and_tolerates_missing_fields() {
        let saved = EventsFilters {
            faults_only: true,
            types: TypeChoice::Warning,
            reason: "BackOff".into(),
            kind: "Pod".into(),
            namespace: "web".into(),
            text: "image".into(),
        };
        let json = serde_json::to_value(&saved).unwrap();
        assert_eq!(
            serde_json::from_value::<EventsFilters>(json).unwrap(),
            saved
        );
        let partial: EventsFilters =
            serde_json::from_value(serde_json::json!({ "text": "x" })).unwrap();
        assert_eq!(partial.text, "x");
        assert!(!partial.faults_only);
    }
}
