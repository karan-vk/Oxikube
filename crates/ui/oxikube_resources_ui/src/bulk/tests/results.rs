//! The results panel: virtualised for thousands of rows, and the retry of the failed ones.

use gpui::TestAppContext;
use oxikube_app::{BulkReport, BulkRow, BulkStatus};
use oxikube_domain::OxiError;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_ports::DeleteOutcome;

use super::*;
use crate::bulk::BulkStage;

fn big_report(n: usize) -> BulkReport {
    BulkReport {
        batch: "selection-1-0".into(),
        dry_run: false,
        verb: "deleted".into(),
        rows: (0..n)
            .map(|i| BulkRow {
                target: ResourceRef::namespaced(
                    cluster(),
                    Gvk::new("", "v1", "Pod"),
                    "x",
                    format!("web-{i}"),
                ),
                status: if i % 100 == 7 {
                    BulkStatus::Failed("no".into())
                } else {
                    BulkStatus::Ok("Deleted".into())
                },
            })
            .collect(),
    }
}

#[gpui::test]
fn thousands_of_rows_draw_only_the_ones_in_view(cx: &mut TestAppContext) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(3));
    let table = f.open_pods();
    select_all(&mut f, &table);
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    d.update(&mut f.vcx, |d, cx| {
        d.set_report_for_test(big_report(5_000));
        cx.notify();
    });
    f.settle();
    assert!(drawn(&mut f, "bulk-result-0"));
    assert!(
        !drawn(&mut f, "bulk-result-4000"),
        "a row far below the list is not drawn"
    );
    let mut count = 0;
    for ix in 0..5_000 {
        let selector: &'static str = Box::leak(format!("bulk-result-{ix}").into_boxed_str());
        if f.vcx.debug_bounds(selector).is_some() {
            count += 1;
        }
    }
    assert!(count > 0 && count < 40, "{count} rows drawn of 5000");
    assert!(drawn(&mut f, "bulk-summary") && drawn(&mut f, "bulk-retry"));
}

#[gpui::test]
fn retry_runs_the_same_action_over_only_the_failed_rows_and_merges_the_results(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::with_actions(cx);
    f.connect_with(pods(4));
    let table = f.open_pods();
    // web-1's rehearsal is refused the first time only.
    let ports = f.ports();
    let script = ports.resources.script();
    script.delete.push_ok(DeleteOutcome::Deleted);
    script.delete.push_ok(DeleteOutcome::Deleted);
    script
        .delete
        .push_err(OxiError::forbidden("pods \"web-1\" is forbidden"));
    select_all(&mut f, &table);
    f.keys(&table, "delete");
    let d = dialog(&mut f).unwrap();
    with_dialog(&mut f, &d, |d, _, cx| d.confirm(cx));
    f.settle();
    let first = d.read_with(&f.vcx, |d, _| d.report().cloned()).unwrap();
    assert_eq!((first.succeeded(), first.failed()), (3, 1));
    let calls_before = f.ports().resources.mutating_calls().len();

    with_dialog(&mut f, &d, |d, _, cx| d.retry_failed(cx));
    f.settle();
    let (stage, second) = d.read_with(&f.vcx, |d, _| (d.stage(), d.report().cloned().unwrap()));
    assert_eq!(stage, BulkStage::Done);
    assert_eq!(second.rows.len(), 4, "one list, the failed row replaced");
    assert_eq!((second.succeeded(), second.failed()), (4, 0));
    assert_ne!(second.batch, first.batch, "the retry is a batch of its own");
    // Only web-1 was sent again: a rehearsal and a delete.
    assert_eq!(f.ports().resources.mutating_calls().len(), calls_before + 2);
    // Nothing failed any more, so no retry is offered.
    assert!(!drawn(&mut f, "bulk-retry"));
    assert!(drawn(&mut f, "bulk-close"));
}
