//! A settings edit reaches an open session live: no reconnect, one update, only for its cluster.

use std::sync::Arc;

use futures::{FutureExt as _, StreamExt as _};
use gpui::{BorrowAppContext as _, TestAppContext};
use oxikube_app::{ClusterSession, ClusterSessionManager, SessionChange, SessionUpdates};
use oxikube_domain::ClusterColour;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_domain::session::SessionPhase;
use oxikube_ports::{ClusterContext, SourceId};
use oxikube_settings::{ClusterSettings, SettingsStore};
use oxikube_testkit::{FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort};

use super::follow_cluster_settings;
use crate::app_state::AppState;

const PROD: &str = "3f2a9c1b7d4e8a60";
const LAB: &str = "0011223344556677";

fn context(id: &str, name: &str) -> ClusterContext {
    ClusterContext {
        cluster: id.parse().unwrap(),
        context: ContextName::new(name),
        source: SourceId("kubeconfig".into()),
        server: None,
        default_namespace: None,
    }
}

fn id(text: &str) -> ClusterId {
    text.parse().unwrap()
}

struct Fixture {
    manager: ClusterSessionManager,
    connector: Arc<FakeClusterConnectorPort>,
    updates: SessionUpdates,
}

fn fixture(cx: &mut TestAppContext) -> (Fixture, gpui::Subscription) {
    let connector = Arc::new(FakeClusterConnectorPort::new());
    let source = Arc::new(
        FakeClusterSourcePort::new().with_contexts([context(PROD, "prod-eu"), context(LAB, "lab")]),
    );
    let manager = ClusterSessionManager::new(
        connector.clone(),
        source,
        Arc::new(FakeClockPort::default()),
    );
    let updates = manager.subscribe();
    let sub = cx.update(|cx| {
        AppState::test(cx);
        follow_cluster_settings(cx, manager.clone())
    });
    (
        Fixture {
            manager,
            connector,
            updates,
        },
        sub,
    )
}

fn edit_settings_file(cx: &mut TestAppContext, text: &str) {
    let text = text.to_owned();
    cx.update(|cx| {
        cx.update_global::<SettingsStore, _>(|store, _| store.set_user_settings(&text).unwrap());
    });
    cx.run_until_parked();
}

fn drain(updates: &mut SessionUpdates) -> Vec<(ClusterId, SessionChange)> {
    let mut out = Vec::new();
    while let Some(Some(Ok(update))) = updates.next().now_or_never() {
        out.push((update.cluster, update.change));
    }
    out
}

fn open_and_connect(f: &Fixture) -> ClusterSession {
    f.manager.open_configured(&context(PROD, "prod-eu"));
    f.manager.open_configured(&context(LAB, "lab"));
    let state = f
        .manager
        .connect(&id(PROD))
        .now_or_never()
        .expect("fake connects at once")
        .unwrap();
    assert_eq!(state.phase(), SessionPhase::Ready);
    f.manager.get(&id(PROD)).unwrap()
}

#[gpui::test]
fn editing_the_file_with_a_new_colour_updates_the_session_without_a_reconnect(
    cx: &mut TestAppContext,
) {
    let (mut f, _sub) = fixture(cx);
    let session = open_and_connect(&f);
    assert_eq!(session.colour(), None);
    drain(&mut f.updates);
    let connects = f.connector.recorded_calls().len();

    edit_settings_file(
        cx,
        &format!(
            r##"{{ "clusters": {{ "{PROD}": {{ "colour": "#e5484d", "display_name": "Production" }} }} }}"##
        ),
    );

    let session = f.manager.get(&id(PROD)).unwrap();
    let red = ClusterColour::rgb(0xe5, 0x48, 0x4d);
    assert_eq!(session.colour(), Some(red));
    assert_eq!(session.title(), "Production");
    assert_eq!(session.phase(), SessionPhase::Ready, "still connected");
    assert_eq!(f.connector.recorded_calls().len(), connects, "no reconnect");
    assert_eq!(f.connector.live_connections(&id(PROD)), 1);
    assert_eq!(
        drain(&mut f.updates),
        [
            (id(PROD), SessionChange::ColourChanged(Some(red))),
            (
                id(PROD),
                SessionChange::DisplayNameChanged(Some("Production".into()))
            ),
        ],
        "one update per changed field, none for the other cluster"
    );

    // And back: removing the block restores the defaults.
    edit_settings_file(cx, "{}");
    let session = f.manager.get(&id(PROD)).unwrap();
    assert_eq!((session.colour(), session.title()), (None, "prod-eu"));
}

#[gpui::test]
async fn the_read_only_toggle_goes_through_the_file_and_back_into_the_session(
    cx: &mut TestAppContext,
) {
    let (mut f, _sub) = fixture(cx);
    open_and_connect(&f);
    drain(&mut f.updates);

    cx.update(|cx| ClusterSettings::set_read_only(cx, &id(PROD), Some("prod-eu"), true))
        .await
        .unwrap();
    cx.run_until_parked();

    assert!(f.manager.get(&id(PROD)).unwrap().read_only());
    assert!(!f.manager.get(&id(LAB)).unwrap().read_only());
    let updates = drain(&mut f.updates);
    assert!(updates.contains(&(id(PROD), SessionChange::ReadOnlyChanged(true))));
    assert!(updates.iter().all(|(cluster, _)| *cluster == id(PROD)));
}

#[gpui::test]
fn a_session_opened_after_the_settings_loaded_starts_from_them(cx: &mut TestAppContext) {
    let (f, _sub) = fixture(cx);
    edit_settings_file(
        cx,
        &format!(
            r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true, "default_namespace": "payments" }} }} }}"#
        ),
    );
    let session = f.manager.open_configured(&context(PROD, "prod-eu"));
    assert!(session.read_only());
    assert_eq!(
        session.prefs().default_namespace.as_deref(),
        Some("payments")
    );
}

#[gpui::test]
fn a_bad_edit_leaves_the_session_as_it_was(cx: &mut TestAppContext) {
    let (mut f, _sub) = fixture(cx);
    edit_settings_file(
        cx,
        &format!(r#"{{ "clusters": {{ "{PROD}": {{ "read_only": true }} }} }}"#),
    );
    open_and_connect(&f);
    drain(&mut f.updates);

    edit_settings_file(
        cx,
        &format!(r#"{{ "clusters": {{ "{PROD}": {{ "read_only": "nope" }} }} }}"#),
    );

    assert!(f.manager.get(&id(PROD)).unwrap().read_only());
    assert!(drain(&mut f.updates).is_empty());
}
