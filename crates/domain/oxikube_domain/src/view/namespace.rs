//! [`NamespaceSummary`] and the warnings shown before a Namespace is deleted (E12-S16).
//!
//! The summary is what the Namespaces panel and the delete dialog read: the phase (a namespace
//! being deleted is `Terminating`, not gone), the finalizers that hold it, the conditions the
//! namespace controller reports about what is left, and the markers of the Hierarchical Namespace
//! Controller (HNC). [`delete_notes`] turns that and the counts of what the namespace holds into
//! the lines of the dialog, most severe first.
//!
//! # HNC
//!
//! HNC (<https://github.com/kubernetes-sigs/hierarchical-namespaces>) marks the namespaces it
//! manages in the namespaces themselves:
//!
//! | Marker | Meaning |
//! |---|---|
//! | annotation `hnc.x-k8s.io/subnamespace-of: <parent>` | the namespace was created from a `SubnamespaceAnchor` in `<parent>` |
//! | label `hnc.x-k8s.io/included-namespace: "true"` | HNC manages the namespace |
//! | label `<ancestor>.tree.hnc.x-k8s.io/depth: "<n>"` | `<ancestor>` is `n` levels above this namespace (`0` is the namespace itself) |
//!
//! A namespace is therefore a descendant of `P` when it carries `P.tree.hnc.x-k8s.io/depth` with a
//! depth above zero. The markers are read from the namespace objects alone, so no HNC custom
//! resource has to be installed or readable.

use std::fmt::Write as _;
use std::sync::Arc;

use jiff::Timestamp;

use super::{Condition, ViewError, arr_of, check_kind, str_of, sub};
use crate::resource::{Resource, StrMap};

/// Annotation HNC puts on a namespace created from a subnamespace anchor; its value is the parent.
pub const HNC_SUBNAMESPACE_OF: &str = "hnc.x-k8s.io/subnamespace-of";
/// Label HNC puts on every namespace it manages.
pub const HNC_INCLUDED: &str = "hnc.x-k8s.io/included-namespace";
/// Suffix of the label that names an ancestor: `<ancestor>.tree.hnc.x-k8s.io/depth`.
pub const HNC_DEPTH_SUFFIX: &str = ".tree.hnc.x-k8s.io/depth";

/// The namespaces the cluster itself needs; deleting one gets an extra warning.
pub const PROTECTED_NAMESPACES: [&str; 4] =
    ["kube-system", "kube-public", "kube-node-lease", "default"];

/// `status.phase` of a Namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamespacePhase {
    /// In use.
    Active,
    /// Being deleted: its contents and finalizers are going away.
    Terminating,
    /// The phase is missing or not one this build knows.
    Unknown,
}

impl NamespacePhase {
    /// The text of the `STATUS` column.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "Active",
            Self::Terminating => "Terminating",
            Self::Unknown => "Unknown",
        }
    }
}

/// How HNC relates a namespace to the others, read from the namespace alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HncInfo {
    /// The parent when the namespace is a subnamespace (created from an anchor).
    pub subnamespace_of: Option<Arc<str>>,
    /// Whether HNC manages the namespace at all.
    pub managed: bool,
}

/// One Namespace, typed: the fields the panel and the delete dialog read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamespaceSummary {
    /// `metadata.name`.
    pub name: Arc<str>,
    /// `status.phase`.
    pub phase: NamespacePhase,
    /// `metadata.labels`.
    pub labels: StrMap,
    /// `metadata.creationTimestamp`.
    pub created: Option<Timestamp>,
    /// `metadata.deletionTimestamp`: when the deletion was requested.
    pub deletion: Option<Timestamp>,
    /// `spec.finalizers`: the finalizers of the namespace controller (normally `kubernetes`).
    pub spec_finalizers: Vec<Arc<str>>,
    /// `metadata.finalizers`: finalizers other controllers added.
    pub finalizers: Vec<Arc<str>>,
    /// `status.conditions`: what the namespace controller says is left (`NamespaceContentRemaining`
    /// and friends; `True` means a problem).
    pub conditions: Vec<Condition>,
    /// HNC markers.
    pub hnc: HncInfo,
}

impl NamespaceSummary {
    /// Build the summary of a core `Namespace`.
    ///
    /// # Errors
    ///
    /// [`ViewError::WrongKind`] when `res` is not a core `Namespace`. Missing or malformed fields
    /// never fail; they fall back to defaults.
    pub fn from_resource(res: &Resource) -> Result<Self, ViewError> {
        check_kind(res, "Namespace", &[("", "Namespace")])?;
        let status = sub(res.json(), "status");
        let phase = match str_of(status, "phase") {
            Some("Active") => NamespacePhase::Active,
            Some("Terminating") => NamespacePhase::Terminating,
            _ if res.meta.deletion.is_some() => NamespacePhase::Terminating,
            _ => NamespacePhase::Unknown,
        };
        let spec_finalizers = arr_of(sub(res.json(), "spec"), "finalizers")
            .iter()
            .filter_map(|f| f.as_str().map(Arc::from))
            .collect();
        let conditions = arr_of(status, "conditions")
            .iter()
            .filter_map(Condition::from_json)
            .collect();
        let hnc = HncInfo {
            subnamespace_of: res.meta.annotations.get(HNC_SUBNAMESPACE_OF).cloned(),
            managed: res
                .meta
                .labels
                .get(HNC_INCLUDED)
                .is_some_and(|v| &**v == "true"),
        };
        Ok(Self {
            name: res.meta.name.clone(),
            phase,
            labels: res.meta.labels.clone(),
            created: res.meta.creation,
            deletion: res.meta.deletion,
            spec_finalizers,
            finalizers: res.meta.finalizers.clone(),
            conditions,
            hnc,
        })
    }

    /// Whether the namespace is being deleted.
    pub fn is_terminating(&self) -> bool {
        self.phase == NamespacePhase::Terminating
    }

    /// The finalizers holding the namespace, controller ones first.
    pub fn all_finalizers(&self) -> impl Iterator<Item = &Arc<str>> {
        self.spec_finalizers.iter().chain(&self.finalizers)
    }

    /// Conditions that are `True`: for a Namespace each one names something that blocks its
    /// deletion (`NamespaceContentRemaining`, `NamespaceFinalizersRemaining`, ...).
    pub fn blockers(&self) -> impl Iterator<Item = &Condition> {
        self.conditions.iter().filter(|c| c.is_true())
    }

    /// A namespace known by name alone (it is not in the cache): phase unknown, nothing else.
    pub fn named(name: &str) -> Self {
        Self {
            name: Arc::from(name),
            phase: NamespacePhase::Unknown,
            labels: StrMap::new(),
            created: None,
            deletion: None,
            spec_finalizers: Vec::new(),
            finalizers: Vec::new(),
            conditions: Vec::new(),
            hnc: HncInfo::default(),
        }
    }

    /// Whether this namespace sits below `ancestor` in an HNC tree.
    pub fn is_hnc_descendant_of(&self, ancestor: &str) -> bool {
        is_hnc_descendant(&self.labels, &self.name, ancestor)
    }

    /// Why deleting this namespace needs extra care, when it is one the cluster depends on.
    pub fn protected(&self) -> Option<&'static str> {
        protected_reason(&self.name)
    }
}

/// Whether the namespace `name` with `labels` sits below `ancestor` in an HNC tree: it carries
/// `<ancestor>.tree.hnc.x-k8s.io/depth` with a depth above zero (zero is the namespace itself).
pub fn is_hnc_descendant(labels: &StrMap, name: &str, ancestor: &str) -> bool {
    if name == ancestor {
        return false;
    }
    // Walks the labels instead of building the `<ancestor>.tree...` key: no allocation, for a
    // check that runs once per namespace in the cache.
    labels.iter().any(|(key, depth)| {
        key.strip_suffix(HNC_DEPTH_SUFFIX) == Some(ancestor)
            && depth.parse::<u32>().is_ok_and(|depth| depth > 0)
    })
}

/// Why the namespace `name` must not be deleted lightly; `None` for an ordinary one.
pub fn protected_reason(name: &str) -> Option<&'static str> {
    Some(match name {
        "kube-system" => {
            "kube-system runs the cluster's own components (DNS, the proxy, controllers): \
             deleting it breaks the cluster."
        }
        "kube-public" => {
            "kube-public holds the cluster information every client may read; \
             deleting it breaks discovery for new clients."
        }
        "kube-node-lease" => {
            "kube-node-lease holds the heartbeat of every node: \
             deleting it makes the nodes look unhealthy."
        }
        "default" => {
            "default is where objects land when no namespace is named: \
             everything in it is lost (the API server recreates it empty)."
        }
        _ => return None,
    })
}

/// How serious a note in the delete dialog is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NoteLevel {
    /// Worth knowing.
    Info,
    /// Check before confirming.
    Warning,
    /// Deleting this can break the cluster.
    Danger,
}

/// One line of the delete dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteNote {
    /// How serious it is.
    pub level: NoteLevel,
    /// The line.
    pub text: String,
}

impl DeleteNote {
    fn new(level: NoteLevel, text: impl Into<String>) -> Self {
        Self {
            level,
            text: text.into(),
        }
    }
}

/// The notes to show before `namespace` is deleted, most severe first.
///
/// `descendants` is how many other namespaces sit below it in an HNC tree; `contents` is what the
/// namespace holds, as (kind, count) for the kinds the app has watched (kinds it has not are
/// absent, not zero). Pure.
pub fn delete_notes(
    namespace: &NamespaceSummary,
    descendants: usize,
    contents: &[(String, u32)],
) -> Vec<DeleteNote> {
    let mut notes = Vec::new();
    if let Some(reason) = namespace.protected() {
        notes.push(DeleteNote::new(NoteLevel::Danger, reason));
    }
    if let Some(parent) = &namespace.hnc.subnamespace_of {
        notes.push(DeleteNote::new(
            NoteLevel::Warning,
            format!(
                "This is an HNC subnamespace of {parent}. HNC owns it through a subnamespace \
                 anchor in {parent}: delete the anchor there instead, or HNC may refuse or \
                 recreate it."
            ),
        ));
    }
    if descendants > 0 {
        notes.push(DeleteNote::new(
            NoteLevel::Warning,
            format!(
                "{descendants} HNC subnamespace{} below it. Deleting a parent removes its \
                 children too when HNC allows cascading deletion.",
                if descendants == 1 { " sits" } else { "s sit" }
            ),
        ));
    }
    if namespace.is_terminating() {
        let mut text = String::from("Already terminating");
        if let Some(since) = namespace.deletion {
            let _ = write!(text, " since {since}");
        }
        let finalizers: Vec<&str> = namespace.all_finalizers().map(|f| f.as_ref()).collect();
        if !finalizers.is_empty() {
            let _ = write!(text, "; waiting for finalizers {}", finalizers.join(", "));
        }
        text.push('.');
        notes.push(DeleteNote::new(NoteLevel::Info, text));
    }
    let held: Vec<String> = contents
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(kind, count)| format!("{count} {kind}{}", if *count == 1 { "" } else { "s" }))
        .collect();
    if !held.is_empty() {
        notes.push(DeleteNote::new(
            NoteLevel::Warning,
            format!("It holds {}; all of it goes with it.", held.join(", ")),
        ));
    }
    notes.sort_by_key(|note| std::cmp::Reverse(note.level));
    notes
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn namespace(mut value: serde_json::Value) -> NamespaceSummary {
        value["apiVersion"] = json!("v1");
        value["kind"] = json!("Namespace");
        let res = Resource::from_json(value).unwrap();
        NamespaceSummary::from_resource(&res).unwrap()
    }

    fn plain(name: &str) -> NamespaceSummary {
        namespace(json!({
            "metadata": { "name": name },
            "spec": { "finalizers": ["kubernetes"] },
            "status": { "phase": "Active" },
        }))
    }

    #[test]
    fn reads_the_phase_and_finalizers() {
        let ns = plain("team-a");
        assert_eq!(ns.phase, NamespacePhase::Active);
        assert_eq!(ns.spec_finalizers, [Arc::from("kubernetes")]);
        assert!(!ns.is_terminating());

        let ns = namespace(json!({
            "metadata": {
                "name": "gone",
                "deletionTimestamp": "2026-01-02T03:04:05Z",
                "finalizers": ["example.com/hold"],
            },
            "spec": { "finalizers": ["kubernetes"] },
            "status": {
                "phase": "Terminating",
                "conditions": [
                    { "type": "NamespaceContentRemaining", "status": "True", "reason": "SomeResourcesRemain" },
                    { "type": "NamespaceDeletionContentFailure", "status": "False" },
                ],
            },
        }));
        assert!(ns.is_terminating());
        assert_eq!(ns.all_finalizers().count(), 2);
        let blockers: Vec<_> = ns.blockers().map(|c| &*c.kind).collect();
        assert_eq!(blockers, ["NamespaceContentRemaining"]);
    }

    #[test]
    fn a_deletion_timestamp_means_terminating_even_without_a_phase() {
        let ns = namespace(json!({
            "metadata": { "name": "x", "deletionTimestamp": "2026-01-02T03:04:05Z" },
        }));
        assert_eq!(ns.phase, NamespacePhase::Terminating);
    }

    #[test]
    fn the_wrong_kind_is_refused() {
        let res = Resource::from_json(
            json!({ "apiVersion": "v1", "kind": "Pod", "metadata": { "name": "p" } }),
        )
        .unwrap();
        assert!(NamespaceSummary::from_resource(&res).is_err());
    }

    #[test]
    fn protected_namespaces_are_the_four_system_ones() {
        for name in PROTECTED_NAMESPACES {
            assert!(plain(name).protected().is_some(), "{name}");
        }
        assert!(plain("team-a").protected().is_none());
        assert!(plain("kube-systems").protected().is_none());
    }

    #[test]
    fn hnc_markers_are_read_from_the_namespace() {
        let child = namespace(json!({
            "metadata": {
                "name": "team-a-dev",
                "annotations": { "hnc.x-k8s.io/subnamespace-of": "team-a" },
                "labels": {
                    "hnc.x-k8s.io/included-namespace": "true",
                    "team-a.tree.hnc.x-k8s.io/depth": "1",
                    "team-a-dev.tree.hnc.x-k8s.io/depth": "0",
                },
            },
        }));
        assert_eq!(child.hnc.subnamespace_of.as_deref(), Some("team-a"));
        assert!(child.hnc.managed);
        assert!(child.is_hnc_descendant_of("team-a"));
        assert!(
            !child.is_hnc_descendant_of("team-a-dev"),
            "depth 0 is itself"
        );
        assert!(!child.is_hnc_descendant_of("other"));
        assert!(!plain("team-a").is_hnc_descendant_of("team-a"));
    }

    #[test]
    fn notes_are_most_severe_first_and_only_say_what_is_known() {
        assert!(delete_notes(&plain("team-a"), 0, &[]).is_empty());

        let notes = delete_notes(
            &plain("kube-system"),
            0,
            &[
                ("Pod".into(), 12),
                ("Deployment".into(), 1),
                ("Job".into(), 0),
            ],
        );
        assert_eq!(notes[0].level, NoteLevel::Danger);
        assert!(notes[0].text.contains("kube-system"));
        let last = &notes[1];
        assert_eq!(last.level, NoteLevel::Warning);
        assert!(last.text.contains("12 Pods, 1 Deployment"), "{}", last.text);
        assert!(!last.text.contains("Job"), "zero counts are left out");
    }

    #[test]
    fn hnc_notes_name_the_parent_and_the_children() {
        let child = namespace(json!({
            "metadata": {
                "name": "team-a-dev",
                "annotations": { "hnc.x-k8s.io/subnamespace-of": "team-a" },
            },
        }));
        let notes = delete_notes(&child, 0, &[]);
        assert_eq!(notes.len(), 1);
        assert!(notes[0].text.contains("subnamespace of team-a"));

        let notes = delete_notes(&plain("team-a"), 2, &[]);
        assert!(notes[0].text.contains("2 HNC subnamespaces sit below"));
        let notes = delete_notes(&plain("team-a"), 1, &[]);
        assert!(notes[0].text.contains("1 HNC subnamespace sits below"));
    }

    #[test]
    fn a_terminating_namespace_says_what_holds_it() {
        let ns = namespace(json!({
            "metadata": { "name": "gone", "deletionTimestamp": "2026-01-02T03:04:05Z" },
            "spec": { "finalizers": ["kubernetes"] },
            "status": { "phase": "Terminating" },
        }));
        let notes = delete_notes(&ns, 0, &[]);
        assert_eq!(notes[0].level, NoteLevel::Info);
        assert!(notes[0].text.contains("2026-01-02"), "{}", notes[0].text);
        assert!(notes[0].text.contains("kubernetes"), "{}", notes[0].text);
    }
}
