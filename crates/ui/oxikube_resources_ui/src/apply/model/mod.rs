//! The panel's state without a window: the stage, the load summary, the rows and what the user
//! did to them (filter, expansion, selection). Pure, so it is tested on its own.

mod lines;
mod row;

use std::collections::BTreeSet;
use std::time::Duration;

use oxikube_app::apply::{ObjectResult, ObjectStatus};
use oxikube_domain::apply::ApplyPlan;
use oxikube_domain::manifests::ManifestError;

pub use lines::{Line, details};
pub use row::Row;

/// Which of the three views of the panel is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The path, how many documents it holds and the ones that did not parse.
    Load,
    /// The plan in apply order with the server's dry run of each object.
    Review,
    /// What the apply did to each object.
    Result,
}

/// What the panel is doing right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    /// Nothing: waiting for the user.
    Idle,
    /// Reading files (or building with kustomize) and ordering the plan.
    Loading,
    /// Asking the server to dry-run each object.
    DryRunning,
    /// Applying.
    Applying,
}

/// What a load found, for the Load view.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LoadSummary {
    /// Manifest files read.
    pub files: usize,
    /// Objects found.
    pub documents: usize,
    /// Documents and files that did not parse, with file and line.
    pub errors: Vec<ManifestError>,
    /// Entries skipped on purpose (symbolic links, a kustomization).
    pub notes: Vec<String>,
    /// The kustomize tool and version that built it, for a kustomize source.
    pub tool: Option<String>,
    /// What the tool wrote to stderr while succeeding.
    pub stderr: String,
}

/// The panel's state. See the [module docs](self).
#[derive(Debug)]
pub struct Model {
    stage: Stage,
    activity: Activity,
    summary: Option<LoadSummary>,
    rows: Vec<Row>,
    errors_only: bool,
    expanded: BTreeSet<usize>,
    selected: Option<usize>,
    lines: Vec<Line>,
    dirty: bool,
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

impl Model {
    /// An empty panel on the Load view.
    pub fn new() -> Self {
        Self {
            stage: Stage::Load,
            activity: Activity::Idle,
            summary: None,
            rows: Vec::new(),
            errors_only: false,
            expanded: BTreeSet::new(),
            selected: None,
            lines: Vec::new(),
            dirty: false,
        }
    }

    /// The view shown.
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// Shows another view.
    pub fn show(&mut self, stage: Stage) {
        self.stage = stage;
    }

    /// What the panel is doing.
    pub fn activity(&self) -> Activity {
        self.activity
    }

    /// Records what the panel is doing.
    pub fn set_activity(&mut self, activity: Activity) {
        self.activity = activity;
    }

    /// Whether work is running.
    pub fn busy(&self) -> bool {
        self.activity != Activity::Idle
    }

    /// A new load starts: the old plan and its results go.
    pub fn begin_load(&mut self) {
        self.stage = Stage::Load;
        self.activity = Activity::Loading;
        self.summary = None;
        self.clear_rows();
    }

    /// Records what the load found.
    pub fn loaded(&mut self, summary: LoadSummary) {
        self.summary = Some(summary);
    }

    /// The last load's summary.
    pub fn summary(&self) -> Option<&LoadSummary> {
        self.summary.as_ref()
    }

    /// The plan is ordered: one row per object, on the Review view.
    pub fn planned(&mut self, plan: &ApplyPlan) {
        self.clear_rows();
        self.rows = plan
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| Row::of(index, item))
            .collect();
        self.stage = Stage::Review;
        self.dirty = true;
    }

    fn clear_rows(&mut self) {
        self.rows.clear();
        self.expanded.clear();
        self.selected = None;
        self.dirty = true;
    }

    /// The apply starts: the Result view, every row waiting.
    pub fn begin_apply(&mut self) {
        self.stage = Stage::Result;
        self.activity = Activity::Applying;
        for row in &mut self.rows {
            row.done = None;
            row.elapsed = None;
        }
        self.dirty = true;
    }

    /// The dry run starts: every row waits for its answer again.
    pub fn begin_dry_run(&mut self) {
        self.stage = Stage::Review;
        self.activity = Activity::DryRunning;
        for row in &mut self.rows {
            row.dry = None;
            row.done = None;
            row.elapsed = None;
        }
        self.dirty = true;
    }

    /// Records one object's result, `elapsed` into the run. A result for an object that is not
    /// in the plan is dropped.
    pub fn record(&mut self, result: ObjectResult, elapsed: Duration) {
        let Some(row) = self.rows.get_mut(result.index) else {
            return;
        };
        if result.dry_run {
            row.dry = Some(result.status);
        } else {
            row.done = Some(result.status);
        }
        row.elapsed = Some(elapsed);
        self.dirty = true;
    }

    /// Marks every object that has no answer yet as skipped, for a run that ended early (a
    /// refusal of the whole plan, or a cancel).
    pub fn skip_unanswered(&mut self, reason: &str) {
        let applying = self.stage == Stage::Result;
        for row in &mut self.rows {
            let slot = if applying {
                &mut row.done
            } else {
                &mut row.dry
            };
            if slot.is_none() {
                *slot = Some(ObjectStatus::Skipped {
                    reason: reason.to_owned(),
                });
            }
        }
        self.dirty = true;
    }

    /// The plan's rows, in apply order.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// Whether only the failed rows are shown.
    pub fn errors_only(&self) -> bool {
        self.errors_only
    }

    /// Shows only the failed and skipped rows, or all of them.
    pub fn set_errors_only(&mut self, on: bool) {
        if self.errors_only != on {
            self.errors_only = on;
            self.dirty = true;
        }
    }

    /// The row selected (by index into [`rows`](Self::rows)).
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Selects row `index`.
    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = Some(index);
        }
    }

    /// Moves the selection through the rows that are shown. `delta` of `isize::MIN` goes to the
    /// first, `isize::MAX` to the last.
    pub fn step(&mut self, delta: isize) {
        let shown: Vec<usize> = self.visible_rows();
        if shown.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|selected| shown.iter().position(|&index| index == selected));
        let target = match (at, delta) {
            (_, isize::MIN) => 0,
            (_, isize::MAX) => shown.len() - 1,
            (None, d) if d >= 0 => 0,
            (None, _) => shown.len() - 1,
            (Some(at), d) => at.saturating_add_signed(d).min(shown.len() - 1),
        };
        self.selected = Some(shown[target]);
    }

    /// Opens or closes the full message of row `index`, when it has one.
    pub fn toggle_expanded(&mut self, index: usize) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        if details(row).is_empty() {
            return;
        }
        if !self.expanded.remove(&index) {
            self.expanded.insert(index);
        }
        self.dirty = true;
    }

    /// Whether row `index` is open.
    pub fn is_expanded(&self, index: usize) -> bool {
        self.expanded.contains(&index)
    }

    fn visible_rows(&self) -> Vec<usize> {
        self.rows
            .iter()
            .filter(|row| !self.errors_only || row.failed())
            .map(|row| row.index)
            .collect()
    }

    /// The table's lines under the filter and the expansions.
    pub fn lines(&mut self) -> &[Line] {
        if self.dirty {
            let mut lines = Vec::with_capacity(self.rows.len());
            for row in &self.rows {
                if self.errors_only && !row.failed() {
                    continue;
                }
                lines.push(Line::Row(row.index));
                if self.expanded.contains(&row.index) {
                    lines.extend(
                        details(row)
                            .into_iter()
                            .map(|text| Line::Detail(row.index, text)),
                    );
                }
            }
            self.lines = lines;
            self.dirty = false;
        }
        &self.lines
    }

    /// How many rows have the status word `word` ([`ObjectStatus::word`]) as shown.
    pub fn count(&self, word: &str) -> usize {
        self.rows
            .iter()
            .filter(|row| row.shown().is_some_and(|status| status.word() == word))
            .count()
    }

    /// "2 created, 1 failed": the non-zero counts; "waiting" before any answer.
    pub fn summary_line(&self) -> String {
        let words = [
            "created",
            "configured",
            "unchanged",
            "deferred",
            "no preview",
            "failed",
            "skipped",
        ];
        let parts: Vec<String> = words
            .iter()
            .filter_map(|word| match self.count(word) {
                0 => None,
                n => Some(format!("{n} {word}")),
            })
            .collect();
        if parts.is_empty() {
            "waiting".to_owned()
        } else {
            parts.join(", ")
        }
    }

    /// How many rows failed or were skipped, as shown.
    pub fn failures(&self) -> usize {
        self.rows.iter().filter(|row| row.failed()).count()
    }

    /// `(kind, name)` of each failed or skipped row: what "Re-run failed only" keeps.
    pub fn failed_keys(&self) -> Vec<(String, String)> {
        self.rows
            .iter()
            .filter(|row| row.failed())
            .map(|row| (row.kind.clone(), row.name.clone()))
            .collect()
    }

    /// The results as plain text, one line per object and its detail lines under it: names,
    /// statuses and the server's messages, never an object's content.
    pub fn as_text(&self) -> String {
        let mut text = String::new();
        for row in &self.rows {
            let word = row.shown().map_or("waiting", ObjectStatus::word);
            text.push_str(&format!("{word}\t{}", row.label()));
            if let Some(elapsed) = row.elapsed {
                text.push_str(&format!("\t{:.1}s", elapsed.as_secs_f32()));
            }
            text.push('\n');
            for detail in details(row) {
                text.push_str(&format!("  {detail}\n"));
            }
        }
        text
    }
}
