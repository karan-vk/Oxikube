//! Paging, filters, selection and the detail panel.

use gpui::TestAppContext;
use oxikube_app::audit::query::{AuditFilters, PAGE_SIZE};
use oxikube_domain::audit::{AuditOutcome, Initiator};
use oxikube_domain::safety::MutationVerb;

use super::{Fx, rec, seed};

#[gpui::test]
fn the_first_page_is_newest_first_and_only_one_page_is_read(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(450));
    fx.open();
    let names = fx.names();
    assert_eq!(names.len(), PAGE_SIZE, "one page, not the log");
    assert_eq!(names[0], "web-449");
    assert_eq!(names[PAGE_SIZE - 1], format!("web-{}", 450 - PAGE_SIZE));
    assert!(fx.model(|m| m.has_more()));
    assert!(fx.drawn("audit-table"));
}

#[gpui::test]
fn the_next_page_is_read_as_the_rows_near_the_end_are_shown(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(450));
    fx.open();
    // The table reports its visible rows after a frame; scrolling to the end of what is loaded
    // asks for the next page.
    fx.with_view(|view, cx| view.table.scroll_to_row(PAGE_SIZE - 1, cx));
    fx.settle();
    assert!(fx.names().len() > PAGE_SIZE, "an older page was appended");
    fx.with_view(|view, cx| view.load_more(cx));
    assert_eq!(fx.names().len(), 450);
    assert!(!fx.model(|m| m.has_more()));
    let mut names = fx.names();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), 450, "every record once");
    let queries = fx.queries();
    assert!(
        queries.iter().skip(1).all(|q| q.cursor.is_some()),
        "keyset pages after the first"
    );
}

#[gpui::test]
fn filters_go_into_the_store_query(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(60));
    fx.open();
    let before = fx.queries().len();
    fx.with_view(|view, cx| {
        view.set_filters(
            AuditFilters {
                verb: Some(MutationVerb::Delete),
                initiator: Some(Initiator::Ui),
                outcome: Some(AuditOutcome::Succeeded),
                cluster: Some(super::cluster()),
                ..AuditFilters::default()
            },
            cx,
        );
    });
    let query = fx.queries().pop().expect("a query");
    assert!(fx.queries().len() > before);
    assert_eq!(query.verb, Some(MutationVerb::Delete));
    assert_eq!(query.initiator, Some(Initiator::Ui));
    assert_eq!(query.outcome, Some(AuditOutcome::Succeeded));
    assert_eq!(query.cluster, Some(super::cluster()));
    let rows = fx.model(|m| m.records().iter().cloned().collect::<Vec<_>>());
    assert_eq!(
        rows.len(),
        10,
        "every sixth record is a successful Ui delete"
    );
    assert!(
        rows.iter()
            .all(|r| r.verb == Some(MutationVerb::Delete) && r.initiator == Initiator::Ui)
    );
}

#[gpui::test]
fn an_older_page_of_a_replaced_filter_is_dropped(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(450));
    fx.open();
    fx.with_view(|view, cx| {
        view.load_more(cx);
        view.set_filters(
            AuditFilters {
                initiator: Some(Initiator::Agent),
                ..AuditFilters::default()
            },
            cx,
        );
    });
    let rows = fx.model(|m| m.records().iter().cloned().collect::<Vec<_>>());
    assert!(
        rows.iter().all(|r| r.initiator == Initiator::Agent),
        "no row of the old filter"
    );
}

#[gpui::test]
fn a_record_opens_its_detail_and_a_batch_lists_its_records(cx: &mut TestAppContext) {
    let mut records = seed(5);
    for n in 0..4 {
        records.push(
            rec(
                100 + n,
                &format!("batch-{n}"),
                "pod::Delete",
                Initiator::Ui,
                AuditOutcome::Succeeded,
            )
            .with_batch_id("b-1"),
        );
    }
    let mut fx = Fx::new(cx, records);
    fx.open();
    assert!(!fx.drawn("audit-detail"));
    // Rows 0..4 are the batch (the newest); row 4 is a plain record.
    fx.with_view(|view, cx| view.select(Some(4), cx));
    assert!(fx.drawn("audit-detail"));
    assert!(fx.drawn("audit-show-resource"));
    assert!(!fx.drawn("audit-batch"), "web-… has no batch");
    fx.with_view(|view, cx| view.select(Some(0), cx));
    assert!(fx.drawn("audit-batch"));
    let batch = fx.with_view(|view, _| {
        view.batch
            .as_ref()
            .map(|b| (b.id.clone(), format!("{:?}", b.load)))
    });
    let (id, load) = batch.expect("the batch was read");
    assert_eq!(&*id, "b-1");
    assert!(
        load.contains("batch-0") && load.contains("batch-3"),
        "{load}"
    );
}

#[gpui::test]
fn show_resource_dispatches_open_for_the_records_object(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.open();
    fx.with_view(|view, cx| view.select(Some(0), cx));
    fx.click("audit-show-resource");
    let sent = fx.dispatcher.sent();
    let Some(oxikube_domain::command::Command::ResourceOpen { target }) = sent.last() else {
        panic!("resource::Open was sent: {sent:?}");
    };
    assert_eq!(&*target.name, "web-2");
}

#[gpui::test]
fn the_cluster_column_is_the_context_name(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(1));
    fx.open();
    let text = fx.with_view(|view, cx| {
        let names = view.table.delegate(cx).names.clone();
        let record = view.model(cx).get(0).cloned().expect("a row");
        crate::audit::model::cell_text(&record, crate::audit::AuditColumn::Cluster, &names)
    });
    assert_eq!(&*text, "kind");
}

#[test]
fn replacing_the_page_hands_back_the_old_records_to_free_off_thread() {
    let mut model = crate::audit::AuditModel::new();
    let _ = model.replace(seed(5), None);
    let old = model.replace(seed(2), None);
    assert_eq!(old.len(), 5, "the caller frees the replaced page");
}

#[test]
fn the_model_never_holds_more_than_its_cap_however_far_the_table_scrolls() {
    use crate::audit::MAX_LOADED_RECORDS;
    let mut model = crate::audit::AuditModel::new();
    let cursor = Some(oxikube_ports::AuditCursor::new(
        jiff::Timestamp::UNIX_EPOCH,
        1,
    ));
    let page = seed(PAGE_SIZE as i64);
    let _ = model.replace(page.clone(), cursor);
    // A 300k-record log scrolled to its end.
    for _ in 1..(300_000 / PAGE_SIZE) {
        if !model.has_more() {
            break;
        }
        model.append(page.clone(), cursor);
    }
    assert_eq!(model.len(), MAX_LOADED_RECORDS);
    assert!(
        model.capped() && !model.has_more(),
        "no more pages are read"
    );

    // Records stored meanwhile go in front; the oldest loaded ones drop off the end.
    model.select(Some(3));
    let selected = model.selected_record().cloned();
    let fresh: Vec<_> = (0..5)
        .map(|n| {
            super::rec(
                1_000_000 + n,
                &format!("new-{n}"),
                "pod::Delete",
                Initiator::Ui,
                AuditOutcome::Succeeded,
            )
        })
        .rev()
        .collect();
    model.prepend(fresh);
    assert_eq!(model.len(), MAX_LOADED_RECORDS);
    assert_eq!(&*model.get(0).expect("a row").target.name, "new-4");
    assert_eq!(model.selected(), Some(8), "the selection keeps its record");
    assert_eq!(model.selected_record().cloned(), selected);

    // A new filter starts over below the cap.
    let _ = model.replace(seed(3), cursor);
    assert!(!model.capped() && model.has_more());
}

#[test]
fn a_log_that_ends_at_the_cap_is_not_capped() {
    use crate::audit::MAX_LOADED_RECORDS;
    let mut model = crate::audit::AuditModel::new();
    let cursor = Some(oxikube_ports::AuditCursor::new(
        jiff::Timestamp::UNIX_EPOCH,
        1,
    ));
    let page = seed(PAGE_SIZE as i64);
    let _ = model.replace(page.clone(), cursor);
    let pages = MAX_LOADED_RECORDS / PAGE_SIZE;
    for n in 1..pages {
        model.append(page.clone(), (n + 1 < pages).then_some(cursor).flatten());
    }
    assert_eq!(model.len(), MAX_LOADED_RECORDS);
    assert!(!model.capped() && !model.has_more(), "the end of the log");
}
