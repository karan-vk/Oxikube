//! The exporters: quoting, unicode, formula neutralisation, and a 100 000-record export that
//! never holds more than a page.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use jiff::{Timestamp, ToSpan as _};
use oxikube_domain::audit::{AuditDenial, AuditOutcome, AuditRecord, Initiator};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{AuditCursor, AuditPage, AuditQuery, StateKey, StatePort, StateTable};
use oxikube_testkit::{FakeFsPort, FakeStatePort};
use serde_json::Value;

use super::*;
use crate::testing::pod;

fn base() -> Timestamp {
    "2026-10-30T12:00:00Z".parse().expect("a valid timestamp")
}

fn record(name: &str, who: &str) -> AuditRecord {
    AuditRecord::new(
        base(),
        who,
        Initiator::Agent,
        "pod::Delete",
        pod("shop", name),
        false,
        AuditOutcome::Succeeded,
    )
}

#[test]
fn leading_formula_characters_are_neutralised() {
    for cell in ["=1+1", "+1", "-2", "@SUM(A1)", "\tx", "\rx"] {
        assert_eq!(neutralise(cell), format!("'{cell}"), "{cell:?}");
    }
    for cell in ["", "plain", "a=b", "1-2", "'=already"] {
        assert!(matches!(neutralise(cell), Cow::Borrowed(_)), "{cell:?}");
    }
}

#[test]
fn csv_quotes_commas_quotes_and_line_breaks() {
    let mut out = String::new();
    write_csv_row(&record("web-0", "say \"hi\", then\nleave"), &mut out);
    assert!(out.contains("\"say \"\"hi\"\", then\nleave\""), "{out}");
    assert!(out.ends_with('\n'));
}

#[test]
fn csv_cells_that_look_like_formulas_are_prefixed() {
    let mut out = String::new();
    write_csv_row(&record("=cmd|' /C calc'!A0", "@evil"), &mut out);
    assert!(out.contains(",'@evil,"), "{out}");
    assert!(
        out.contains("\"'=cmd|' /C calc'!A0\"") || out.contains("'=cmd|' /C calc'!A0"),
        "{out}"
    );
    assert!(!out.contains(",=cmd"), "{out}");
}

#[test]
fn csv_keeps_unicode_and_has_the_documented_columns() {
    let mut out = String::new();
    let mut r = record("web-0", "日本語 ユーザー");
    r.denial = Some(AuditDenial::ReadOnly);
    r.outcome = AuditOutcome::Denied;
    write_csv_row(&r, &mut out);
    assert!(out.contains("日本語 ユーザー"));
    assert!(out.contains(",denied,agent,false,"));
    assert!(out.contains(",read_only,"));
    assert_eq!(CSV_HEADER.split(',').count(), 14);
    assert_eq!(out.trim_end().split(',').count(), 14);
}

#[test]
fn jsonl_is_one_record_per_line_and_reads_back() {
    let records = vec![record("web-0", "alice"), record("web-1", "bob")];
    let text = format_page(AuditExportFormat::Jsonl, &records, true);
    let back: Vec<AuditRecord> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("each line is a record"))
        .collect();
    assert_eq!(back, records);
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).expect("json");
        let keys = value.as_object().expect("an object");
        assert!(!keys.contains_key("body"), "records carry no bodies");
    }
}

#[test]
fn only_csv_has_a_header_and_only_on_the_first_page() {
    let records = vec![record("web-0", "alice")];
    assert!(format_page(AuditExportFormat::Csv, &records, true).starts_with("time,cluster"));
    assert!(!format_page(AuditExportFormat::Csv, &records, false).starts_with("time,cluster"));
    assert!(!format_page(AuditExportFormat::Jsonl, &records, true).contains("time,cluster"));
}

#[test]
fn the_suggested_name_has_the_extension() {
    assert_eq!(
        suggested_file_name(AuditExportFormat::Csv, base()),
        "oxikube-audit-20261030-120000.csv"
    );
    assert!(suggested_file_name(AuditExportFormat::Jsonl, base()).ends_with(".jsonl"));
}

/// A store that makes up `total` records on demand and remembers the largest page it was asked
/// for: proof the export never reads more than a page.
struct Synthetic {
    total: u64,
    cluster: ClusterId,
    largest_page: parking_lot::Mutex<usize>,
}

#[async_trait]
impl StatePort for Synthetic {
    async fn kv_get(&self, _: &StateKey) -> OxiResult<Option<Value>> {
        Ok(None)
    }
    async fn kv_set(&self, _: &StateKey, _: Value) -> OxiResult<()> {
        Ok(())
    }
    async fn kv_delete(&self, _: &StateKey) -> OxiResult<bool> {
        Ok(false)
    }
    async fn kv_list(&self, _: &str) -> OxiResult<Vec<(StateKey, Value)>> {
        Ok(Vec::new())
    }
    async fn table_get(&self, _: &StateTable, _: &StateKey) -> OxiResult<Option<Value>> {
        Ok(None)
    }
    async fn table_put(&self, _: &StateTable, _: &StateKey, _: Value) -> OxiResult<()> {
        Ok(())
    }
    async fn table_delete(&self, _: &StateTable, _: &StateKey) -> OxiResult<bool> {
        Ok(false)
    }
    async fn table_list(
        &self,
        _: &StateTable,
        _: Option<usize>,
    ) -> OxiResult<Vec<(StateKey, Value)>> {
        Ok(Vec::new())
    }
    async fn append_audit(&self, _: &[AuditRecord]) -> OxiResult<()> {
        Ok(())
    }
    async fn query_audit(&self, query: &AuditQuery) -> OxiResult<AuditPage> {
        let mut largest = self.largest_page.lock();
        *largest = (*largest).max(query.limit);
        // Rows count down from `total`; the cursor names the last row of the previous page.
        let from = query.cursor.map_or(self.total + 1, |c| c.row());
        let to = from.saturating_sub(query.limit as u64).max(1);
        let records: Vec<_> = (to..from)
            .rev()
            .map(|row| {
                let target = ResourceRef::namespaced(
                    self.cluster.clone(),
                    Gvk::from_api_version("v1", "Pod"),
                    "shop",
                    format!("web-{row}"),
                );
                AuditRecord::new(
                    base() + (row as i64).seconds(),
                    "alice",
                    Initiator::Ui,
                    "pod::Delete",
                    target,
                    false,
                    AuditOutcome::Succeeded,
                )
            })
            .collect();
        let next = (to > 1).then(|| AuditCursor::new(base() + (to as i64).seconds(), to));
        Ok(AuditPage { records, next })
    }
    async fn prune_audit(&self, _: Timestamp) -> OxiResult<u64> {
        Ok(0)
    }
}

fn synthetic(total: u64) -> Arc<Synthetic> {
    Arc::new(Synthetic {
        total,
        cluster: ClusterId::new("kubeconfig", &ContextName::new("prod")),
        largest_page: parking_lot::Mutex::new(0),
    })
}

#[tokio::test]
async fn a_hundred_thousand_records_stream_in_bounded_chunks() {
    for format in AuditExportFormat::ALL {
        let state = synthetic(100_000);
        let (mut stream, counters) = chunks(
            AuditReader::new(state.clone()),
            AuditFilters::default(),
            base(),
            format,
        );
        let (mut chunk_count, mut biggest, mut lines) = (0usize, 0usize, 0usize);
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.expect("a chunk");
            biggest = biggest.max(chunk.len());
            lines += chunk.iter().filter(|b| **b == b'\n').count();
            chunk_count += 1;
        }
        let header = usize::from(format == AuditExportFormat::Csv);
        assert_eq!(lines, 100_000 + header, "{format:?}");
        assert_eq!(chunk_count, 100_000 / EXPORT_PAGE, "one chunk per page");
        assert!(
            biggest < 512 * 1024,
            "a chunk is a page, not the log: {biggest}"
        );
        assert_eq!(*state.largest_page.lock(), EXPORT_PAGE);
        assert_eq!(counters.records.load(Ordering::Relaxed), 100_000);
    }
}

#[tokio::test]
async fn export_writes_the_filtered_log_through_the_fs_port() {
    let state = Arc::new(FakeStatePort::new());
    let records: Vec<_> = (0..30)
        .map(|n| {
            let outcome = if n % 2 == 0 {
                AuditOutcome::Succeeded
            } else {
                AuditOutcome::Failed
            };
            let mut r = record(&format!("web-{n}"), "alice");
            r.ts = base() + n.seconds();
            r.outcome = outcome;
            r
        })
        .collect();
    state.append_audit(&records).await.expect("seeds");
    let fs = FakeFsPort::new();
    let path = PathBuf::from("/tmp/audit.csv");
    let filters = AuditFilters {
        outcome: Some(AuditOutcome::Failed),
        ..AuditFilters::default()
    };
    let summary = export(
        &fs,
        &path,
        AuditReader::new(state),
        filters,
        base(),
        AuditExportFormat::Csv,
    )
    .await
    .expect("exports");
    assert_eq!(summary.records, 15);
    let text = String::from_utf8(fs.file(&path).expect("written")).expect("utf-8");
    assert_eq!(text.lines().count(), 16, "header and 15 rows");
    assert!(text.lines().skip(1).all(|line| line.contains(",failed,")));
    assert_eq!(summary.bytes as usize, text.len());
}

#[tokio::test]
async fn a_failing_store_abandons_the_export() {
    let state = Arc::new(FakeStatePort::new());
    state
        .script()
        .query_audit
        .push_err(OxiError::internal("database is locked"));
    let fs = FakeFsPort::new();
    let path = PathBuf::from("/tmp/audit.jsonl");
    let result = export(
        &fs,
        &path,
        AuditReader::new(state),
        AuditFilters::default(),
        base(),
        AuditExportFormat::Jsonl,
    )
    .await;
    assert!(result.is_err());
    assert!(fs.file(&path).is_none(), "nothing is left behind");
}
