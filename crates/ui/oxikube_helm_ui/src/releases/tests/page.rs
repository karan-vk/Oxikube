//! The table: rows, sort, filter, selection, the detail opening, native-only mode, keys.

use gpui::TestAppContext;
use oxikube_domain::ErrorKind;
use oxikube_domain::helm::{HelmCapabilities, ReleaseColumn, ReleaseStatus};
use oxikube_ui::table::SortDirection;

use super::{Data, Harness, every_status, full, release};
use crate::releases::{
    CloseDetail, Load, NextTab, Phase, PreviousTab, SelectFirst, SelectLast, SelectNext,
    SelectPrevious, Tab,
};

fn data(releases: Vec<oxikube_domain::helm::Release>) -> Data {
    Data {
        releases,
        ..Data::default()
    }
}

#[gpui::test]
fn the_table_lists_a_row_per_release_for_every_status(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(every_status()), full());
    assert!(h.is_laid_out("helm-releases"));
    assert!(h.is_laid_out("helm-releases-table"));
    assert!(h.is_laid_out("helm-releases-toolbar"));
    assert_eq!(h.names().len(), ReleaseStatus::ALL.len());
    h.read(|v, cx| {
        v.table.read(cx, |d| {
            assert_eq!(d.total(), 9);
            assert_eq!(*d.phase(), Phase::Ready);
            for (ix, status) in ReleaseStatus::ALL.into_iter().enumerate() {
                let release = d.release(ix).expect("row");
                assert_eq!(release.status, status);
            }
        });
    });
    // The epic's columns, in order.
    let titles = h.read(|v, cx| {
        use oxikube_ui::TableDelegate as _;
        v.table.read(cx, |d| {
            (0..d.columns_count(cx))
                .map(|ix| d.column(ix, cx).name.to_string())
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(
        titles,
        [
            "Name",
            "Namespace",
            "Chart",
            "Revision",
            "Version",
            "App version",
            "Status",
            "Updated"
        ]
    );
    assert_eq!(h.calls(), ["list"], "one native list, nothing else decoded");
}

#[gpui::test]
fn cells_read_name_namespace_chart_revision_versions_status(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![release("apps", "web", ReleaseStatus::Failed, 7)]),
        full(),
    );
    let cells = h.read(|v, cx| {
        use oxikube_ui::TableDelegate as _;
        v.table.read(cx, |d| {
            (0..8)
                .map(|col| d.cell_text(0, col, cx))
                .collect::<Vec<_>>()
        })
    });
    assert_eq!(
        &cells[..7],
        ["web", "apps", "web", "7", "1.2.3", "2.0.0", "failed"]
    );
    assert!(
        cells[7].ends_with('y') || cells[7].ends_with('d'),
        "an age: {}",
        cells[7]
    );
}

#[gpui::test]
fn sorting_by_a_column_reorders_the_rows_and_unsorting_restores_them(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![
            release("b", "zeta", ReleaseStatus::Deployed, 3),
            release("a", "alpha", ReleaseStatus::Failed, 1),
            release("a", "mid", ReleaseStatus::Deployed, 2),
        ]),
        full(),
    );
    let original = h.names();
    let table = h.read(|v, _| v.table.clone());
    // What a header click does: the table calls the delegate's `perform_sort`, which sets this.
    let sort = |h: &mut Harness, column: ReleaseColumn, direction| {
        let table = table.clone();
        h.vcx.update(|_, cx| {
            table.update(cx, |d| {
                d.set_sort(match direction {
                    SortDirection::Unsorted => None,
                    SortDirection::Ascending => Some((column, false)),
                    SortDirection::Descending => Some((column, true)),
                });
            });
        });
        h.settle();
    };
    sort(&mut h, ReleaseColumn::Revision, SortDirection::Descending);
    assert_eq!(h.names(), ["b/zeta", "a/mid", "a/alpha"]);
    sort(&mut h, ReleaseColumn::Status, SortDirection::Ascending);
    assert_eq!(h.names(), ["a/mid", "b/zeta", "a/alpha"]);
    sort(&mut h, ReleaseColumn::Name, SortDirection::Unsorted);
    assert_eq!(h.names(), original);
    assert!(h.read(|v, cx| v.table.read(cx, |d| d.sort())).is_none());
}

#[gpui::test]
fn the_filter_uses_the_resource_tables_grammar(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, data(every_status()), full());
    let view = h.view.clone();
    let filter = |h: &mut Harness, text: &str| {
        let text = text.to_owned();
        h.vcx
            .update(|_, cx| view.update(cx, |v, cx| v.set_filter_text(&text, cx)));
        h.settle();
        h.names().len()
    };
    assert_eq!(filter(&mut h, "failed"), 1);
    assert_eq!(filter(&mut h, "pending"), 3);
    assert_eq!(filter(&mut h, "!pending"), 6);
    assert_eq!(filter(&mut h, "^rel-1-"), 1);
    assert_eq!(filter(&mut h, "-f rl4"), 1);
    assert_eq!(filter(&mut h, ""), 9);
    // A broken regular expression filters nothing and says so.
    assert_eq!(filter(&mut h, "("), 9);
    assert!(h.is_laid_out("helm-releases-filter-problem"));
    assert_eq!(filter(&mut h, "nothing-matches-this"), 0);
    assert!(h.is_laid_out("helm-releases-empty"));
}

#[gpui::test]
fn selecting_a_row_opens_the_detail_at_once_with_a_skeleton(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![release("apps", "web", ReleaseStatus::Deployed, 2)]),
        full(),
    );
    assert!(!h.is_laid_out("helm-release-detail"));
    let view = h.view.clone();
    // Selected, but nothing has been read yet: the header and the skeleton state are there.
    h.vcx
        .update(|_, cx| view.update(cx, |v, cx| v.select_row(0, cx)));
    h.read(|v, _| {
        let detail = v.detail.as_ref().expect("opened in the same update");
        assert_eq!(detail.tab, Tab::Resources);
        assert_eq!(*detail.load(Tab::Resources), Load::Loading);
        assert_eq!(
            *detail.load(Tab::Values),
            Load::Idle,
            "other tabs read when opened"
        );
    });
    h.settle();
    assert!(h.is_laid_out("helm-release-detail"));
    assert!(h.is_laid_out("helm-release-title"));
    for tab in Tab::ALL {
        assert!(
            h.is_laid_out(&format!("helm-release-tab:{}", tab.key())),
            "{tab:?}"
        );
    }
    let calls = h.calls();
    assert_eq!(
        calls,
        ["list", "objects:web:2"],
        "only the shown tab is read"
    );
}

#[gpui::test]
fn a_click_on_a_row_selects_it_and_follows_it_through_a_sort(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![
            release("apps", "a", ReleaseStatus::Deployed, 1),
            release("apps", "b", ReleaseStatus::Deployed, 2),
        ]),
        full(),
    );
    h.select(1);
    let selected = h.read(|v, cx| v.table.read(cx, |d| d.selected().cloned()));
    assert_eq!(selected.map(|r| r.name), Some("b".to_owned()));
    let table = h.read(|v, _| v.table.clone());
    h.vcx.update(|_, cx| {
        table.update(cx, |d| d.set_sort(Some((ReleaseColumn::Name, true))));
    });
    let row = h.read(|v, cx| {
        v.table
            .read(cx, |d| d.selected().and_then(|r| d.position_of(r)))
    });
    assert_eq!(row, Some(0), "the selection is the release, not the line");
}

#[gpui::test]
fn keys_move_the_selection_and_the_tabs_and_close_the_detail(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![
            release("apps", "a", ReleaseStatus::Deployed, 1),
            release("apps", "b", ReleaseStatus::Deployed, 1),
            release("apps", "c", ReleaseStatus::Deployed, 1),
        ]),
        full(),
    );
    let name =
        |h: &mut Harness| h.read(|v, _| v.selected_release().map(|r| r.release.name.clone()));
    let view = h.view.clone();
    let focus = h.read(|v, _| v.focus.clone());
    h.vcx.update(|window, cx| window.focus(&focus, cx));
    h.vcx.dispatch_action(SelectNext);
    h.settle();
    assert_eq!(
        name(&mut h).as_deref(),
        Some("a"),
        "the first row when none is selected"
    );
    h.vcx.dispatch_action(SelectNext);
    h.vcx.dispatch_action(SelectNext);
    h.vcx.dispatch_action(SelectNext);
    h.settle();
    assert_eq!(name(&mut h).as_deref(), Some("c"), "stops at the last row");
    h.vcx.dispatch_action(SelectPrevious);
    h.settle();
    assert_eq!(name(&mut h).as_deref(), Some("b"));
    h.vcx.dispatch_action(SelectFirst);
    h.settle();
    assert_eq!(name(&mut h).as_deref(), Some("a"));
    h.vcx.dispatch_action(SelectLast);
    h.settle();
    assert_eq!(name(&mut h).as_deref(), Some("c"));

    h.vcx.dispatch_action(NextTab);
    h.settle();
    assert_eq!(h.detail(|d| d.tab), Tab::Values);
    h.vcx.dispatch_action(PreviousTab);
    h.vcx.dispatch_action(PreviousTab);
    h.settle();
    assert_eq!(h.detail(|d| d.tab), Tab::History, "tabs wrap");
    h.vcx.dispatch_action(CloseDetail);
    h.settle();
    assert!(h.read(|v, _| v.detail.is_none()));
    assert!(!h.is_laid_out("helm-release-detail"));
    drop(view);
}

#[gpui::test]
fn native_only_mode_works_and_shows_no_mutation_controls(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![release("apps", "web", ReleaseStatus::Deployed, 2)]),
        HelmCapabilities::none(),
    );
    h.select(0);
    assert_eq!(h.names(), ["apps/web"], "the list works without a binary");
    assert!(h.is_laid_out("helm-release-detail"));
    assert!(
        h.is_laid_out("helm-release-read-only"),
        "says why it cannot change anything"
    );
    assert!(
        !h.is_laid_out("helm-release-actions"),
        "no mutation slot without helm"
    );
    assert!(h.is_laid_out("helm-banner"), "with guidance");
}

#[gpui::test]
fn with_helm_the_mutation_slot_is_there_and_the_note_is_not(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![release("apps", "web", ReleaseStatus::Deployed, 2)]),
        full(),
    );
    h.select(0);
    assert!(h.is_laid_out("helm-release-actions"));
    assert!(!h.is_laid_out("helm-release-read-only"));
}

#[gpui::test]
fn the_slot_follows_the_binary_appearing(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        data(vec![release("apps", "web", ReleaseStatus::Deployed, 2)]),
        HelmCapabilities::none(),
    );
    h.select(0);
    assert!(!h.is_laid_out("helm-release-actions"));
    let gate = h.gate.clone();
    h.vcx
        .update(|_, cx| gate.update(cx, |g, cx| g.set_cli_capabilities(full(), cx)));
    h.settle();
    assert!(h.is_laid_out("helm-release-actions"));
}

#[gpui::test]
fn a_failed_list_says_why_and_retry_lists_again(cx: &mut TestAppContext) {
    let mut h = Harness::open(
        cx,
        Data {
            fail_list: Some((ErrorKind::Forbidden, "secrets is forbidden".into())),
            ..Data::default()
        },
        full(),
    );
    assert!(h.is_laid_out("helm-releases-empty"));
    h.read(|v, cx| {
        let phase = v.table.read(cx, |d| d.phase().clone());
        assert!(
            matches!(&phase, Phase::Failed(why) if why.contains("not allowed")),
            "{phase:?}"
        );
    });
    assert!(h.is_laid_out("helm-releases-retry"));
    {
        let mut data = h.backend.data.lock();
        data.fail_list = None;
        data.releases = vec![release("apps", "web", ReleaseStatus::Deployed, 1)];
    }
    h.click("helm-releases-retry");
    assert_eq!(h.names(), ["apps/web"]);
    assert!(!h.is_laid_out("helm-releases-retry"));
}

#[gpui::test]
fn an_empty_cluster_says_so(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, Data::default(), full());
    assert!(h.is_laid_out("helm-releases-empty"));
    assert!(h.is_laid_out("helm-releases-count"));
}
