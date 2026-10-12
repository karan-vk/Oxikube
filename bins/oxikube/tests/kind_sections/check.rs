//! The checks of one kind against the cluster, and the custom resources' printer columns.

use oxikube::app_state::AppState;
use oxikube_app::command_bus::{DispatchContext, Outcome};
use oxikube_app::{DispatchError, SubjectKind};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::safety::ConfirmTier;
use oxikube_testkit::sections::{KindCase, column_label};
use oxikube_workspace::sidebar::DEFINITIONS_ENTRY;

use crate::kind_common::Launched;
use crate::{app, cluster};

/// What every kind's rows offer; a kind's own actions are the rest.
const GENERIC: &[CommandId] = &[
    CommandId::RESOURCE_VIEW_YAML,
    CommandId::RESOURCE_VIEW_YAML_IN_EDITOR,
    CommandId::RESOURCE_VIEW_DESCRIBE,
    CommandId::RESOURCE_EDIT,
    CommandId::RESOURCE_DELETE,
];

/// The table columns' titles (a wide one suffixed `*`).
fn columns(
    app: &mut Launched,
    table: &gpui::Entity<oxikube_resources_ui::table::ResourceTable>,
) -> Vec<String> {
    app.vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |d| {
            d.layout()
                .columns()
                .map(|(c, _)| column_label(&c.title, c.wide))
                .collect()
        })
    })
}

/// `case`'s object `name`: listed with the documented columns, its own actions on its row, its
/// detail with its panel, and its mutations guarded without anything changing.
pub fn kind(app: &mut Launched, context: &str, namespace: &str, case: &KindCase, name: &str) {
    let entry = if case.section == "custom-resources" {
        DEFINITIONS_ENTRY.to_owned()
    } else {
        case.entry()
    };
    let table = app::open_table(app, &entry, &case.gvk(), name);

    // The documented columns; one namespace is selected, so a namespaced kind's table leaves its
    // Namespace column out.
    let expected: Vec<&str> = case
        .columns
        .iter()
        .copied()
        .filter(|c| !(case.namespaced() && *c == "Namespace"))
        .collect();
    assert_eq!(columns(app, &table), expected, "{}'s columns", case.kind);
    app.vcx.update(|window, cx| window.draw(cx).clear(cx));

    // The row's menu: the object under the cursor, the kind's own actions.
    let target = if case.namespaced() {
        ResourceRef::namespaced(app.cluster.clone(), case.gvk(), namespace, name)
    } else {
        ResourceRef::cluster_scoped(app.cluster.clone(), case.gvk(), name)
    };
    let (targets, entries) = app.vcx.update(|_, cx| {
        let table = table.read(cx);
        (table.action_targets(cx), table.action_entries(cx))
    });
    assert_eq!(
        targets,
        std::slice::from_ref(&target),
        "{}'s row",
        case.kind
    );
    // `resource::ViewMetrics` is offered by kind family (those with history graphs), not as a
    // kind's own action.
    let metrics = entries
        .iter()
        .any(|e| e.command() == CommandId::RESOURCE_VIEW_METRICS);
    assert_eq!(
        metrics,
        SubjectKind::of(&case.gvk()).is_some(),
        "{}'s View Metrics row action",
        case.kind
    );
    // `selection::Label` ("Edit labels", E12-S19) is offered on every kind the server can patch,
    // which every seeded kind is, not as a kind's own action.
    assert!(
        entries
            .iter()
            .any(|e| e.command() == CommandId::SELECTION_LABEL),
        "{}'s Edit labels row action",
        case.kind
    );
    let own: Vec<CommandId> = entries
        .iter()
        .map(|e| e.command())
        .filter(|id| {
            !GENERIC.contains(id)
                && *id != CommandId::RESOURCE_VIEW_METRICS
                && *id != CommandId::SELECTION_LABEL
        })
        .collect();
    assert_eq!(own, case.actions, "{}'s own row actions", case.kind);

    // The detail reads the object from the cluster and draws with the kind's panel.
    let view = app::open_detail(app, &target);
    let panel = app.vcx.update(|_, cx| view.read(cx).kind_panel().is_some());
    assert_eq!(panel, case.panel, "{}'s panel", case.kind);

    // Every mutation of the row asks first, and a read-only cluster refuses it; nothing changes.
    let before = cluster::fingerprint(context, case, namespace, name);
    let sessions = app.vcx.read(AppState::global).services().sessions.clone();
    for entry in entries.iter().filter(|e| e.action.meta().mutating) {
        let id = entry.command();
        let command = entry.action.command_for(&target);
        let asked = app::dispatch(
            app,
            command.clone(),
            DispatchContext::new(Initiator::Ui, "kind-sections"),
        );
        let Ok(Outcome::NeedsConfirmation(request)) = asked else {
            panic!(
                "{id} on a {}: expected a confirmation, got {asked:?}",
                case.kind
            );
        };
        assert_eq!(request.command, id);
        assert!(
            request.tier >= entry.action.meta().confirm && request.tier != ConfirmTier::None,
            "{id} on a {}: {:?}",
            case.kind,
            request.tier
        );
        sessions
            .set_read_only(&app.cluster, true)
            .expect("the session");
        for initiator in [Initiator::Ui, Initiator::Agent] {
            let refused = app::dispatch(
                app,
                command.clone(),
                DispatchContext::new(initiator, "kind-sections"),
            );
            assert!(
                matches!(refused, Err(DispatchError::ReadOnly { .. })),
                "{id} on a read-only {} for {initiator}: {refused:?}",
                case.kind
            );
        }
        sessions
            .set_read_only(&app.cluster, false)
            .expect("the session");
    }
    let after = cluster::fingerprint(context, case, namespace, name);
    assert_eq!(before, after, "the {} was not changed", case.kind);
}

/// The printer-column parity (epic criterion 4): a Widget of the sample CRD (`cargo xtask
/// kind-up`), opened from its Custom Resources group, has the columns the API server prints for
/// `kubectl get widgets` (the wide `Owner` one too).
pub fn printer_columns(app: &mut Launched, context: &str, namespace: &str) {
    let name = "seed-widget";
    if !cluster::create_widget(context, namespace, name) {
        println!("skipping the printer columns: test.oxikube.dev/v1 Widget is not served");
        return;
    }
    app::activate(app, "crd:test.oxikube.dev");
    let table = app::open_table(
        app,
        "crd:test.oxikube.dev/widgets",
        &Gvk::new("test.oxikube.dev", "v1", "Widget"),
        name,
    );
    let server = cluster::server_columns(
        context,
        &format!("/apis/test.oxikube.dev/v1/namespaces/{namespace}/widgets"),
    );
    assert_eq!(server.first().map(String::as_str), Some("Name"));
    assert_eq!(
        columns(app, &table),
        server,
        "the Widget table is `kubectl get`'s"
    );
    app.vcx.update(|window, cx| window.draw(cx).clear(cx));
}
