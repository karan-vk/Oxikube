//! [`SelectionSummary`]: what a selection holds, for the bulk confirmation.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::ids::ResourceRef;

/// How many objects of one kind live in one namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCount {
    /// The kind (`Pod`).
    pub kind: Arc<str>,
    /// The namespace; `None` for cluster-scoped objects.
    pub namespace: Option<Arc<str>>,
    /// How many.
    pub count: usize,
}

/// The objects of a selection, counted by kind and namespace.
///
/// A bulk confirmation lists these counts instead of every name, so it stays one short dialog for
/// a selection of thousands (the results list is where each object is named).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectionSummary {
    total: usize,
    groups: Vec<GroupCount>,
}

impl SelectionSummary {
    /// Counts `targets`. Groups come out ordered by kind, then namespace (cluster-scoped first).
    pub fn of(targets: &[ResourceRef]) -> Self {
        let mut counts: BTreeMap<(Arc<str>, Option<Arc<str>>), usize> = BTreeMap::new();
        for target in targets {
            *counts
                .entry((target.gvk.kind.clone(), target.namespace.clone()))
                .or_default() += 1;
        }
        Self {
            total: targets.len(),
            groups: counts
                .into_iter()
                .map(|((kind, namespace), count)| GroupCount {
                    kind,
                    namespace,
                    count,
                })
                .collect(),
        }
    }

    /// How many objects.
    pub fn total(&self) -> usize {
        self.total
    }

    /// The counts by kind and namespace.
    pub fn groups(&self) -> &[GroupCount] {
        &self.groups
    }

    /// How many objects of each kind, in kind order.
    pub fn kinds(&self) -> Vec<(Arc<str>, usize)> {
        let mut out: Vec<(Arc<str>, usize)> = Vec::new();
        for group in &self.groups {
            match out.last_mut() {
                Some((kind, count)) if *kind == group.kind => *count += group.count,
                _ => out.push((group.kind.clone(), group.count)),
            }
        }
        out
    }

    /// How many distinct namespaces the objects are in (cluster-scoped objects count for none).
    pub fn namespace_count(&self) -> usize {
        let mut seen: Vec<&Arc<str>> = self
            .groups
            .iter()
            .filter_map(|group| group.namespace.as_ref())
            .collect();
        seen.sort();
        seen.dedup();
        seen.len()
    }

    /// One line per kind: `3 Pods in default, 2 in kube-system` or `1 Namespace`.
    ///
    /// At most `max_lines` lines; the rest are folded into a last `…and N more kinds` line.
    pub fn lines(&self, max_lines: usize) -> Vec<String> {
        let mut by_kind: Vec<(Arc<str>, Vec<&GroupCount>)> = Vec::new();
        for group in &self.groups {
            match by_kind.last_mut() {
                Some((kind, groups)) if *kind == group.kind => groups.push(group),
                _ => by_kind.push((group.kind.clone(), vec![group])),
            }
        }
        let mut lines: Vec<String> = by_kind
            .iter()
            .map(|(kind, groups)| kind_line(kind, groups))
            .collect();
        if lines.len() > max_lines {
            let more = lines.len() - max_lines;
            lines.truncate(max_lines);
            lines.push(format!("…and {more} more kinds"));
        }
        lines
    }
}

/// `3 Pods in default, 2 in kube-system`.
fn kind_line(kind: &str, groups: &[&GroupCount]) -> String {
    let total: usize = groups.iter().map(|g| g.count).sum();
    let noun = if total == 1 {
        kind.to_owned()
    } else {
        format!("{kind}s")
    };
    let spaced: Vec<&&GroupCount> = groups.iter().filter(|g| g.namespace.is_some()).collect();
    if spaced.is_empty() {
        return format!("{total} {noun}");
    }
    let places: Vec<String> = spaced
        .iter()
        .enumerate()
        .map(|(ix, group)| {
            let namespace = group.namespace.as_deref().unwrap_or_default();
            if ix == 0 {
                format!("{} {noun} in {namespace}", group.count)
            } else {
                format!("{} in {namespace}", group.count)
            }
        })
        .collect();
    places.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ClusterId, ContextName, Gvk};

    fn cluster() -> ClusterId {
        ClusterId::new("~/.kube/config", &ContextName::new("kind"))
    }

    fn pod(ns: &str, name: &str) -> ResourceRef {
        ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), ns, name)
    }

    #[test]
    fn counts_by_kind_and_namespace() {
        let mut targets = vec![
            pod("default", "a"),
            pod("default", "b"),
            pod("kube-system", "c"),
        ];
        targets.push(ResourceRef::cluster_scoped(
            cluster(),
            Gvk::new("", "v1", "Namespace"),
            "dev",
        ));
        let summary = SelectionSummary::of(&targets);
        assert_eq!(summary.total(), 4);
        assert_eq!(summary.namespace_count(), 2);
        assert_eq!(
            summary.kinds(),
            vec![(Arc::from("Namespace"), 1), (Arc::from("Pod"), 3)]
        );
        assert_eq!(
            summary.lines(10),
            vec![
                "1 Namespace".to_owned(),
                "2 Pods in default, 1 in kube-system".to_owned()
            ]
        );
    }

    #[test]
    fn a_long_list_folds_into_a_last_line() {
        let targets: Vec<ResourceRef> = ["A", "B", "C", "D"]
            .iter()
            .map(|kind| {
                ResourceRef::namespaced(cluster(), Gvk::new("x.io", "v1", *kind), "ns", "o")
            })
            .collect();
        let lines = SelectionSummary::of(&targets).lines(2);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[2], "…and 2 more kinds");
    }

    #[test]
    fn an_empty_selection_has_nothing() {
        let summary = SelectionSummary::of(&[]);
        assert_eq!(summary.total(), 0);
        assert!(summary.lines(5).is_empty());
    }
}
