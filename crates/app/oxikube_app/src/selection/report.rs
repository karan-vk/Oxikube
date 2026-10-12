//! [`BulkReport`]: what a bulk command did to each object, as data a view draws and an agent
//! reads.

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{OxiError, OxiResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// What happened to one object of a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "message", rename_all = "snake_case")]
pub enum BulkStatus {
    /// Done (or, for a dry run, accepted by the server), with a short word for the list.
    Ok(String),
    /// Left alone, and why (it is gone already).
    Skipped(String),
    /// Refused or failed, as the server said.
    Failed(String),
}

impl BulkStatus {
    /// Whether the row went through.
    pub fn is_ok(&self) -> bool {
        matches!(self, BulkStatus::Ok(_))
    }

    /// Whether the row failed (a skipped row did not).
    pub fn is_failed(&self) -> bool {
        matches!(self, BulkStatus::Failed(_))
    }

    /// The text that goes with the status.
    pub fn message(&self) -> &str {
        match self {
            BulkStatus::Ok(m) | BulkStatus::Skipped(m) | BulkStatus::Failed(m) => m,
        }
    }

    /// A short word for the list: `Done`, `Skipped`, `Failed`.
    pub fn label(&self) -> &'static str {
        match self {
            BulkStatus::Ok(_) => "Done",
            BulkStatus::Skipped(_) => "Skipped",
            BulkStatus::Failed(_) => "Failed",
        }
    }
}

/// One object of a batch and what happened to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BulkRow {
    /// The object.
    pub target: ResourceRef,
    /// What happened.
    #[serde(flatten)]
    pub status: BulkStatus,
}

/// The result of a bulk command: one row per object, in the order asked.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BulkReport {
    /// The id every audit record of the batch carries (`batch=<id>`).
    pub batch: String,
    /// Whether this was a dry run: nothing changed.
    pub dry_run: bool,
    /// What the batch did, in a word for a sentence: `deleted`, `restarted`, `labelled`.
    pub verb: String,
    /// One row per object.
    pub rows: Vec<BulkRow>,
}

impl BulkReport {
    /// How many rows went through.
    pub fn succeeded(&self) -> usize {
        self.rows.iter().filter(|row| row.status.is_ok()).count()
    }

    /// How many rows were left alone.
    pub fn skipped(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| matches!(row.status, BulkStatus::Skipped(_)))
            .count()
    }

    /// How many rows failed.
    pub fn failed(&self) -> usize {
        self.rows
            .iter()
            .filter(|row| row.status.is_failed())
            .count()
    }

    /// The objects whose row failed: what a retry sends again.
    pub fn failed_targets(&self) -> Vec<ResourceRef> {
        self.rows
            .iter()
            .filter(|row| row.status.is_failed())
            .map(|row| row.target.clone())
            .collect()
    }

    /// One line: `Deleted 8 of 10 objects, 1 skipped, 1 failed`, or `Dry run: 10 objects can be
    /// deleted`.
    pub fn summary(&self) -> String {
        let total = self.rows.len();
        let noun = if total == 1 { "object" } else { "objects" };
        let mut line = if self.dry_run {
            format!("Dry run: {} {noun} can be {}", self.succeeded(), self.verb)
        } else if self.succeeded() == total {
            format!("{} {total} {noun}", capitalised(&self.verb))
        } else {
            format!(
                "{} {} of {total} {noun}",
                capitalised(&self.verb),
                self.succeeded()
            )
        };
        if self.skipped() > 0 {
            line.push_str(&format!(", {} skipped", self.skipped()));
        }
        if self.failed() > 0 {
            line.push_str(&format!(", {} failed", self.failed()));
        }
        line
    }

    /// This report with `later` laid over it: each row of `later` (a retry of the failed ones)
    /// replaces the row of the same object, every other row stays, and the batch id is the later
    /// one's. Rows of `later` that this report lacks are appended.
    #[must_use]
    pub fn merged_with(&self, later: &BulkReport) -> BulkReport {
        let mut rows = self.rows.clone();
        let index: std::collections::HashMap<&ResourceRef, usize> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| (&row.target, ix))
            .collect();
        for row in &later.rows {
            match index.get(&row.target) {
                Some(&ix) => rows[ix] = row.clone(),
                None => rows.push(row.clone()),
            }
        }
        BulkReport {
            batch: later.batch.clone(),
            dry_run: later.dry_run,
            verb: later.verb.clone(),
            rows,
        }
    }

    /// The `CommandOutput` data of the report. Rows carry the target and the status only.
    pub fn to_data(&self) -> Value {
        json!({
            "outcome": if self.dry_run { "dry_run" } else { "done" },
            "batch": self.batch,
            "dry_run": self.dry_run,
            "verb": self.verb,
            "ok": self.succeeded(),
            "skipped": self.skipped(),
            "failed": self.failed(),
            "rows": self.rows,
        })
    }

    /// The report a handler's output data describes.
    ///
    /// # Errors
    ///
    /// `Internal` when the data is not a report (a handler that is not a bulk one).
    pub fn from_data(data: Option<&Value>) -> OxiResult<Self> {
        let data = data.ok_or_else(|| OxiError::internal("the batch gave no report"))?;
        serde_json::from_value(data.clone())
            .map_err(|error| OxiError::internal(format!("the batch report is malformed: {error}")))
    }
}

fn capitalised(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk};

    use super::*;

    fn row(name: &str, status: BulkStatus) -> BulkRow {
        BulkRow {
            target: ResourceRef::namespaced(
                ClusterId::new("~/.kube/config", &ContextName::new("kind")),
                Gvk::new("", "v1", "Pod"),
                "default",
                name,
            ),
            status,
        }
    }

    fn report() -> BulkReport {
        BulkReport {
            batch: "selection-1-0".into(),
            dry_run: false,
            verb: "deleted".into(),
            rows: vec![
                row("a", BulkStatus::Ok("Deleted".into())),
                row("b", BulkStatus::Skipped("already gone".into())),
                row("c", BulkStatus::Failed("forbidden".into())),
            ],
        }
    }

    #[test]
    fn counts_and_summary() {
        let report = report();
        assert_eq!(
            (report.succeeded(), report.skipped(), report.failed()),
            (1, 1, 1)
        );
        assert_eq!(
            report.summary(),
            "Deleted 1 of 3 objects, 1 skipped, 1 failed"
        );
        assert_eq!(report.failed_targets().len(), 1);
        assert_eq!(&*report.failed_targets()[0].name, "c");
        let all_ok = BulkReport {
            rows: vec![row("a", BulkStatus::Ok("Deleted".into()))],
            ..report
        };
        assert_eq!(all_ok.summary(), "Deleted 1 object");
    }

    #[test]
    fn a_retry_replaces_only_the_rows_it_ran() {
        let first = report();
        let retry = BulkReport {
            batch: "selection-2-0".into(),
            rows: vec![row("c", BulkStatus::Ok("Deleted".into()))],
            ..first.clone()
        };
        let merged = first.merged_with(&retry);
        assert_eq!(merged.batch, "selection-2-0");
        let names: Vec<_> = merged
            .rows
            .iter()
            .map(|r| r.target.name.to_string())
            .collect();
        assert_eq!(names, ["a", "b", "c"]);
        assert_eq!(
            (merged.succeeded(), merged.skipped(), merged.failed()),
            (2, 1, 0)
        );
    }

    #[test]
    fn a_dry_run_says_what_could_be_done() {
        let dry = BulkReport {
            dry_run: true,
            rows: vec![
                row("a", BulkStatus::Ok("ok".into())),
                row("b", BulkStatus::Ok("ok".into())),
            ],
            ..report()
        };
        assert_eq!(dry.summary(), "Dry run: 2 objects can be deleted");
    }

    #[test]
    fn the_report_survives_the_command_output() {
        let report = report();
        let data = report.to_data();
        assert_eq!(data["failed"], 1);
        assert_eq!(data["rows"][2]["status"], "failed");
        assert_eq!(data["rows"][2]["message"], "forbidden");
        assert_eq!(BulkReport::from_data(Some(&data)).unwrap(), report);
        assert!(BulkReport::from_data(None).is_err());
        assert!(BulkReport::from_data(Some(&json!({"nope": 1}))).is_err());
    }
}
