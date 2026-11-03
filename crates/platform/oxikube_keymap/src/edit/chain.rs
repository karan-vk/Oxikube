//! Conflicts across the context chain: the bindings in force, in every layer, that a new binding
//! would clash with (E21-S08).
//!
//! Two bindings clash when their keystrokes are the same, or one sequence starts the other
//! (`g` makes GPUI wait before it runs `g g`'s first key), *and* their contexts can be active at
//! once. The second half is decided against the focus stacks of [`crate::stacks`]: the two
//! predicates clash when some stack, with some setting of the flags and values they test, matches
//! both at some depth, exactly as GPUI's own matching (`KeyBindingContextPredicate::depth_of`)
//! would. Equal contexts and parent / child contexts (`Workspace` and `ResourceTable`) clash;
//! siblings (`ResourceTable` and `Terminal`) and contradictory flags (`Editing` and `!Editing`)
//! do not. A binding without a context clashes with everything that can be active anywhere.
//!
//! The existing [`crate::conflicts`] report is narrower (one layer, one context); this one is
//! what the recorder shows before a binding is saved.

use std::collections::{BTreeMap, BTreeSet};

use gpui::{KeyBindingContextPredicate, KeyContext};

use super::ops::normalise_context;
use crate::conflicts::normalise;
use crate::known::KnownNames;
use crate::layer::KeymapLayer;
use crate::rows::{KeymapRow, RowStatus};
use crate::stacks::ContextStacks;

/// The flag and value combinations tried per stack before a pair of predicates is assumed to
/// clash (a predicate testing a dozen flags is not worth proving disjoint).
const MAX_ASSIGNMENTS: usize = 1024;

/// How the keys of a binding in force meet the new binding's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    /// The same keystrokes.
    SameKeys,
    /// The new keys start the other binding's sequence (`g` against `g g`).
    ShadowsLonger,
    /// The other binding's keys start the new sequence (`g g` against `g`).
    ShadowedByShorter,
}

/// A binding in force that the new binding clashes with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChainConflict {
    /// The binding, as the keymap editor lists it (action, context, keys, layer).
    pub row: KeymapRow,
    /// How the keys meet.
    pub kind: ConflictKind,
    /// Whether saving the new binding overwrites this one whatever the user answers: a user
    /// binding of the same keys in the same context. A section of `keymap.json` holds one action
    /// per keystrokes, so the two cannot both be kept there ("Keep both" does not apply to it).
    pub overwritten: bool,
}

/// What the conflict check knows about contexts: where they sit and which names are flags.
#[derive(Clone, Debug)]
pub struct ContextModel {
    stacks: ContextStacks,
    known: KnownNames,
}

impl ContextModel {
    /// The registered stacks and names.
    pub fn current() -> Self {
        Self {
            stacks: ContextStacks::current(),
            known: KnownNames::current(),
        }
    }

    /// Exactly `stacks` and `known`, for tests.
    pub fn of(stacks: ContextStacks, known: KnownNames) -> Self {
        Self { stacks, known }
    }

    /// Whether a binding limited to `a` and one limited to `b` (`None`: everywhere) can be
    /// active at the same time. A context that does not parse never loads, so it clashes with
    /// nothing.
    pub fn can_be_active_together(&self, a: Option<&str>, b: Option<&str>) -> bool {
        let (Ok(a), Ok(b)) = (predicate(a), predicate(b)) else {
            return false;
        };
        let mut names = Names::default();
        for p in [&a, &b].into_iter().flatten() {
            names.collect(p, &self.known);
        }
        let mut stacks = self.stacks.clone();
        for context in &names.contexts {
            stacks.ensure(context);
        }
        let assignments = names.assignments();
        let Some(assignments) = assignments else {
            return true;
        };
        stacks.iter().any(|stack| {
            assignments.iter().any(|assignment| {
                let contexts = build_stack(stack, assignment);
                matches(a.as_ref(), &contexts) && matches(b.as_ref(), &contexts)
            })
        })
    }

    /// The bindings in force among `rows` that `keystrokes` for `action` in `context` would clash
    /// with, in row order. A row equal to `editing` (the binding being changed) is left out, and
    /// so is the same action already on the same keys. A user binding of the same keys in the
    /// same context is marked [`ChainConflict::overwritten`]: the write puts the new action in
    /// its place.
    pub fn conflicts(
        &self,
        action: &str,
        context: Option<&str>,
        keystrokes: &str,
        rows: &[KeymapRow],
        editing: Option<&KeymapRow>,
    ) -> Vec<ChainConflict> {
        let keys: Vec<String> = normalise(keystrokes)
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        if keys.is_empty() {
            return Vec::new();
        }
        let place = normalise_context(context);
        let mut together: BTreeMap<Option<&str>, bool> = BTreeMap::new();
        let mut found = Vec::new();
        for row in rows {
            if row.status != RowStatus::Active || row.keystrokes.is_empty() {
                continue;
            }
            if editing.is_some_and(|editing| editing == row) {
                continue;
            }
            let Some(kind) = compare(&keys, &row.keystrokes) else {
                continue;
            };
            if kind == ConflictKind::SameKeys && row.action == action {
                continue;
            }
            let overlap = *together
                .entry(row.context.as_deref())
                .or_insert_with(|| self.can_be_active_together(context, row.context.as_deref()));
            if overlap {
                let overwritten = kind == ConflictKind::SameKeys
                    && row.source == Some(KeymapLayer::User)
                    && normalise_context(row.context.as_deref()) == place;
                found.push(ChainConflict {
                    row: row.clone(),
                    kind,
                    overwritten,
                });
            }
        }
        found
    }
}

fn predicate(context: Option<&str>) -> Result<Option<KeyBindingContextPredicate>, ()> {
    match context.map(str::trim).filter(|c| !c.is_empty()) {
        None => Ok(None),
        Some(text) => KeyBindingContextPredicate::parse(text)
            .map(Some)
            .map_err(|_| ()),
    }
}

fn matches(predicate: Option<&KeyBindingContextPredicate>, contexts: &[KeyContext]) -> bool {
    predicate.is_none_or(|predicate| predicate.depth_of(contexts).is_some())
}

fn compare(new: &[String], other: &[String]) -> Option<ConflictKind> {
    let shared = new.len().min(other.len());
    if new[..shared] != other[..shared] {
        return None;
    }
    Some(match new.len().cmp(&other.len()) {
        std::cmp::Ordering::Equal => ConflictKind::SameKeys,
        std::cmp::Ordering::Less => ConflictKind::ShadowsLonger,
        std::cmp::Ordering::Greater => ConflictKind::ShadowedByShorter,
    })
}

/// The names two predicates test: view contexts, flags, and value keys with the values named.
#[derive(Default)]
struct Names {
    contexts: BTreeSet<String>,
    flags: BTreeSet<String>,
    values: BTreeMap<String, BTreeSet<String>>,
}

/// One setting of the flags and values: the flags that are set, and each key's value (absent
/// when `None`).
type Assignment = (Vec<String>, Vec<(String, Option<String>)>);

impl Names {
    fn collect(&mut self, predicate: &KeyBindingContextPredicate, known: &KnownNames) {
        match predicate {
            KeyBindingContextPredicate::Identifier(name) => {
                let name = name.to_string();
                if known.is_flag(&name) {
                    self.flags.insert(name);
                } else {
                    self.contexts.insert(name);
                }
            }
            KeyBindingContextPredicate::Equal(key, value)
            | KeyBindingContextPredicate::NotEqual(key, value) => {
                self.values
                    .entry(key.to_string())
                    .or_default()
                    .insert(value.to_string());
            }
            KeyBindingContextPredicate::Not(inner) => self.collect(inner, known),
            KeyBindingContextPredicate::Descendant(a, b)
            | KeyBindingContextPredicate::And(a, b)
            | KeyBindingContextPredicate::Or(a, b) => {
                self.collect(a, known);
                self.collect(b, known);
            }
        }
    }

    /// Every setting of the flags (on or off) and keys (absent, a value named, or another
    /// value); `None` when there are more than [`MAX_ASSIGNMENTS`].
    fn assignments(&self) -> Option<Vec<Assignment>> {
        let mut out: Vec<Assignment> = vec![(Vec::new(), Vec::new())];
        for flag in &self.flags {
            let mut next = Vec::with_capacity(out.len() * 2);
            for (flags, values) in &out {
                next.push((flags.clone(), values.clone()));
                let mut on = flags.clone();
                on.push(flag.clone());
                next.push((on, values.clone()));
            }
            out = next;
            if out.len() > MAX_ASSIGNMENTS {
                return None;
            }
        }
        for (key, named) in &self.values {
            let choices: Vec<Option<String>> = std::iter::once(None)
                .chain(named.iter().cloned().map(Some))
                // A value no predicate names, so `key != v` can hold with the key present.
                .chain(std::iter::once(Some("\u{1}other".to_owned())))
                .collect();
            let mut next = Vec::with_capacity(out.len() * choices.len());
            for (flags, values) in &out {
                for choice in &choices {
                    let mut values = values.clone();
                    values.push((key.clone(), choice.clone()));
                    next.push((flags.clone(), values));
                }
            }
            out = next;
            if out.len() > MAX_ASSIGNMENTS {
                return None;
            }
        }
        Some(out)
    }
}

/// The context stack for `names` with `assignment` on every entry (GPUI's `!` looks at the whole
/// stack, and a flag tested with a context is tested on that context's entry, so setting a flag
/// everywhere or nowhere covers both).
fn build_stack(names: &[String], (flags, values): &Assignment) -> Vec<KeyContext> {
    names
        .iter()
        .map(|name| {
            let mut context = KeyContext::default();
            context.add(name.clone());
            for flag in flags {
                context.add(flag.clone());
            }
            for (key, value) in values {
                if let Some(value) = value {
                    context.set(key.clone(), value.clone());
                }
            }
            context
        })
        .collect()
}
