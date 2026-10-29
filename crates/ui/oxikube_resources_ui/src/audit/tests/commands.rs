//! `audit::Open` and `audit::Export` through the bus.

use std::path::PathBuf;

use gpui::TestAppContext;
use oxikube_domain::audit::{AuditExportFormat, AuditRecord};
use oxikube_domain::command::{Command, CommandId};

use super::{Fx, cluster, other_cluster, seed};

#[gpui::test]
fn open_shows_the_viewer_and_opening_again_shows_the_same_one(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    let first = fx.open();
    fx.run(Command::AuditOpen { cluster: None });
    let second = fx.view().expect("open");
    assert_eq!(first.entity_id(), second.entity_id());
    let count = fx.workspace.read_with(&fx.vcx, |ws, _| {
        ws.items_of_type::<crate::audit::AuditView>().len()
    });
    assert_eq!(count, 1);
}

#[gpui::test]
fn open_with_a_cluster_presets_the_cluster_filter(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(3));
    fx.run(Command::AuditOpen {
        cluster: Some(cluster()),
    });
    let filters = fx.with_view(|v, _| v.filters().clone());
    assert_eq!(filters.cluster, Some(cluster()));
    assert_eq!(fx.names().len(), 3);
    fx.run(Command::AuditOpen {
        cluster: Some(other_cluster()),
    });
    assert!(fx.names().is_empty(), "no record of the other cluster");
}

#[gpui::test]
fn export_writes_the_filtered_records_to_the_chosen_file(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(30));
    fx.open();
    fx.with_view(|view, cx| {
        view.set_filters(
            oxikube_app::audit::query::AuditFilters {
                initiator: Some(oxikube_domain::audit::Initiator::Agent),
                ..Default::default()
            },
            cx,
        );
    });
    fx.run(Command::AuditExport {
        format: AuditExportFormat::Jsonl,
    });
    assert!(fx.vcx.did_prompt_for_new_path());
    let path = PathBuf::from("/tmp/out/audit.jsonl");
    let chosen = path.clone();
    fx.vcx.simulate_new_path_selection(move |_| Some(chosen));
    fx.settle();
    let text = String::from_utf8(fx.fs.file(&path).expect("written")).expect("utf-8");
    let records: Vec<AuditRecord> = text
        .lines()
        .map(|line| serde_json::from_str(line).expect("a record per line"))
        .collect();
    assert_eq!(records.len(), 15, "the filtered log, not the loaded page");
    assert!(
        records
            .iter()
            .all(|r| r.initiator == oxikube_domain::audit::Initiator::Agent)
    );
}

#[gpui::test]
fn export_as_csv_has_a_header_and_cancelling_writes_nothing(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(4));
    fx.open();
    fx.run(Command::AuditExport {
        format: AuditExportFormat::Csv,
    });
    fx.vcx.simulate_new_path_selection(|_| None);
    fx.settle();
    assert!(fx.fs.file("/tmp/out/audit.csv").is_none());
    fx.run(Command::AuditExport {
        format: AuditExportFormat::Csv,
    });
    let path = PathBuf::from("/tmp/out/audit.csv");
    let chosen = path.clone();
    fx.vcx.simulate_new_path_selection(move |_| Some(chosen));
    fx.settle();
    let text = String::from_utf8(fx.fs.file(&path).expect("written")).expect("utf-8");
    assert!(text.starts_with("time,cluster,namespace,verb"));
    assert_eq!(text.lines().count(), 5);
}

#[gpui::test]
fn the_toolbars_export_goes_through_the_bus(cx: &mut TestAppContext) {
    let mut fx = Fx::new(cx, seed(2));
    fx.open();
    assert!(fx.drawn("audit-export"));
    fx.with_view(|view, cx| {
        view.deps.dispatcher.dispatch(
            Command::AuditExport {
                format: AuditExportFormat::Csv,
            },
            cx,
        );
    });
    assert_eq!(
        fx.dispatcher.sent().last(),
        Some(&Command::AuditExport {
            format: AuditExportFormat::Csv
        })
    );
}

#[test]
fn the_commands_are_declared_read_only_with_tool_stubs() {
    for (id, tool) in [
        (CommandId::AUDIT_OPEN, "app.audit_open"),
        (CommandId::AUDIT_EXPORT, "app.audit_export"),
    ] {
        let meta = oxikube_domain::command::lookup(id).expect("declared");
        assert!(!meta.mutating, "{id} changes no cluster");
        assert_eq!(id.tool_name(), tool);
    }
}
