//! [`SidebarTree`]: the registered kinds by sidebar section, in rank order.

use std::sync::Arc;

use super::gate::{KindGate, Shown};
use super::meta::{KindMeta, Section};

/// One kind under a section.
#[derive(Clone, Debug)]
pub struct TreeEntry {
    /// The kind's spec.
    pub meta: Arc<KindMeta>,
    /// Its rank within the section.
    pub rank: u32,
}

/// A section and its kinds, in order.
#[derive(Clone, Debug)]
pub struct TreeSection {
    /// The section.
    pub section: Section,
    /// The kinds, by rank, then title, then group.
    pub entries: Vec<TreeEntry>,
}

/// The sidebar as the registry describes it: sections by rank, each with its kinds by rank.
/// Sections without a shown kind are left out.
#[derive(Clone, Debug, Default)]
pub struct SidebarTree {
    /// The sections, by rank (ties by id).
    pub sections: Vec<TreeSection>,
}

impl SidebarTree {
    pub(super) fn build<'a>(
        metas: impl Iterator<Item = &'a Arc<KindMeta>>,
        gate: Option<&KindGate<'_>>,
    ) -> Self {
        let mut sections: Vec<TreeSection> = Vec::new();
        for meta in metas {
            let Some(slot) = meta.sidebar else {
                continue;
            };
            if gate.is_some_and(|gate| gate.judge(&meta.gvk, meta.scope) != Shown::Yes) {
                continue;
            }
            let entry = TreeEntry {
                meta: meta.clone(),
                rank: slot.rank,
            };
            match sections
                .iter_mut()
                .find(|s| s.section.id == slot.section.id)
            {
                Some(section) => section.entries.push(entry),
                None => sections.push(TreeSection {
                    section: slot.section,
                    entries: vec![entry],
                }),
            }
        }
        sections
            .sort_by(|a, b| (a.section.rank, a.section.id).cmp(&(b.section.rank, b.section.id)));
        for section in &mut sections {
            section.entries.sort_by(|a, b| {
                (a.rank, &a.meta.title, &a.meta.gvk.group).cmp(&(
                    b.rank,
                    &b.meta.title,
                    &b.meta.gvk.group,
                ))
            });
        }
        Self { sections }
    }

    /// The section `id`.
    pub fn section(&self, id: &str) -> Option<&TreeSection> {
        self.sections.iter().find(|s| s.section.id == id)
    }

    /// Every kind, in sidebar order.
    pub fn entries(&self) -> impl Iterator<Item = (&Section, &TreeEntry)> {
        self.sections
            .iter()
            .flat_map(|s| s.entries.iter().map(move |e| (&s.section, e)))
    }

    /// How many kinds the tree holds.
    pub fn len(&self) -> usize {
        self.sections.iter().map(|s| s.entries.len()).sum()
    }

    /// Whether the tree holds no kind.
    pub fn is_empty(&self) -> bool {
        self.sections.iter().all(|s| s.entries.is_empty())
    }

    /// The kinds whose title, kind or plural contains `query` (ignoring case), in sidebar order.
    /// An empty query matches everything.
    pub fn search(&self, query: &str) -> Vec<(&Section, &TreeEntry)> {
        let query = query.trim().to_lowercase();
        self.entries()
            .filter(|(_, entry)| {
                let meta = &entry.meta;
                query.is_empty()
                    || [&*meta.title, &*meta.gvk.kind, &*meta.plural]
                        .iter()
                        .any(|text| text.to_lowercase().contains(&query))
            })
            .collect()
    }
}
