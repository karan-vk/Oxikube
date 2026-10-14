//! The events chain in the running app on kind (E14-S10): a pod whose image cannot be pulled, in
//! a namespace of the test's own, makes the kubelet write `Failed` Warnings, and the app shows
//! them everywhere a user looks, all from the cluster's one events feed:
//!
//! 1. `:ev <namespace>` from the jump bar opens the events view; `ctrl-z` shows only the faults.
//!    The Warning is in the view within a second of a plain `kubectl` watch seeing it (epic
//!    done-when 1: the kubelet's own timing drops out).
//! 2. The pods table's warning column badges the pod (`Warning: Failed`), and
//!    `resource::ViewEvents` (the badge's click) opens the detail drawer on the pod's Events tab,
//!    which lists it.
//! 3. The notification centre lists the warning and raises it as a toast (epic done-when 4).
//! 4. After `notifications::MuteCluster` a new failing pod is in the events view but notifies
//!    nothing; after `notifications::UnmuteCluster` the next one notifies again.
//!
//! The service-level halves (exact burst counts, the `core/v1` fallback when `events.k8s.io` is
//! unavailable) are `oxikube_app`'s `kind_smoke::events_chain`.
//!
//! `cargo test -p oxikube --features integration --test kind_events` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. The pods and the namespace
//! are the test's own (`oxi-test-<rand>`, deleted at the end); no node is touched. Other suites
//! share the cluster's events feed, so every assertion is about this test's pods.
#![cfg(feature = "integration")]

#[path = "kind_common/mod.rs"]
mod kind_common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::notifications::{Notification, NotificationRules, NotificationService};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::event::EventType;
use oxikube_domain::ids::ClusterId;
use oxikube_events_ui::EventsView;
use oxikube_resources_ui::detail::{DetailDrawer, DetailTab};
use oxikube_resources_ui::table::ResourceTable;
use oxikube_resources_ui::warnings::BADGE_COLUMN;
use oxikube_testkit::integration::events::{
    EventArrivals, PULL_FAILED_REASON, create, unpullable_pod,
};
use oxikube_testkit::integration::{TestNamespace, ensure_kind_context, test_context};
use oxikube_ui::TableDelegate as _;
use oxikube_workspace::ClusterTab;

use kind_common::{DEADLINE, Launched, launch, wait, wait_value, workspace};

/// The budget from a `kubectl` watch seeing an event to the events view showing it.
const LIVE_WITHIN: Duration = Duration::from_secs(1);

/// How long a muted cluster is given to (not) notify about a warning the view already shows.
const QUIET: Duration = Duration::from_secs(3);

fn dispatch(vcx: &mut VisualTestContext, command: Command) {
    let bus = vcx
        .update(|_, cx| AppState::global(cx).command_bus().cloned())
        .expect("the bus");
    let done = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "kind test")));
    assert!(done.is_ok(), "{done:?}");
}

/// The rows of the centre about the pod `name` in `namespace`.
fn rows_about(centre: &NotificationService, namespace: &str, name: &str) -> Vec<Arc<Notification>> {
    centre
        .list()
        .into_iter()
        .filter(|row| {
            row.target.as_ref().is_some_and(|t| {
                t.name.as_ref() == name && t.namespace.as_deref() == Some(namespace)
            })
        })
        .collect()
}

/// The events view's rows about the pod `name`, as `(type, reason, uid)`.
fn view_rows(
    vcx: &mut VisualTestContext,
    view: &Entity<EventsView>,
    name: &str,
) -> Vec<(EventType, String, String)> {
    vcx.update(|_, cx| {
        view.read(cx)
            .model(cx)
            .events()
            .filter(|e| &*e.regarding.name == name)
            .map(|e| {
                (
                    e.event_type,
                    e.reason.to_string(),
                    e.uid.as_deref().unwrap_or_default().to_owned(),
                )
            })
            .collect()
    })
}

/// What the view holds, for a failure message.
fn view_text(vcx: &mut VisualTestContext, view: &Entity<EventsView>) -> String {
    vcx.update(|_, cx| {
        let view = view.read(cx);
        let rows: Vec<String> = view
            .model(cx)
            .events()
            .map(|e| {
                format!(
                    "{:?} {} {}/{}",
                    e.event_type, e.reason, e.regarding.gvk.kind, e.regarding.name
                )
            })
            .collect();
        format!(
            "feed {:?}, filters {:?}\n{}",
            view.feed_state(),
            view.filters(),
            rows.join("\n")
        )
    })
}

/// The pods table's warning cell for the pod `name` in `namespace` (empty without a badge), once
/// the table lists the pod.
fn badge_text(
    vcx: &mut VisualTestContext,
    table: &Entity<ResourceTable>,
    namespace: &str,
    name: &str,
) -> Option<String> {
    vcx.update(|_, cx| {
        table.read(cx).read_rows(cx, |rows| {
            let row = rows.rows().iter().position(|r| {
                let meta = r.meta();
                &*meta.name == name && meta.namespace.as_deref() == Some(namespace)
            })?;
            let column = rows
                .layout()
                .visible_index(&oxikube_app::ColumnId::new(BADGE_COLUMN))?;
            Some(rows.cell_text(row, column, cx))
        })
    })
}

/// Opens the events view of `namespace` from the jump bar, as a user types it.
fn open_events_view(
    vcx: &mut VisualTestContext,
    tab: &Entity<ClusterTab>,
    namespace: &str,
) -> Entity<EventsView> {
    vcx.simulate_keystrokes(":");
    vcx.run_until_parked();
    vcx.simulate_input(&format!("ev {namespace}"));
    vcx.run_until_parked();
    vcx.simulate_keystrokes("enter");
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let view = wait_value(vcx, "`:ev` to open the events view", |vcx| {
        vcx.update(|_, cx| EventsView::find(&inner, cx))
    });
    wait(vcx, "the view to show the namespace", |vcx| {
        vcx.update(|_, cx| view.read(cx).filters().namespace == namespace)
    });
    view
}

#[gpui::test]
fn a_failing_pull_reaches_the_view_the_badge_the_drawer_and_a_toast(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();

    let ns = TestNamespace::create(&context).expect("namespace");
    let Launched {
        mut vcx,
        tab,
        cluster,
        table,
        dir,
    } = launch(cx, &context);
    let state = vcx.update(|_, cx| AppState::global(cx));
    let centre = state.services().notifications.clone();
    wait(&mut vcx, "the centre to watch the cluster", |_| {
        centre.is_watching(&cluster)
    });
    // Other suites' warnings share the raise limit on this cluster; it is not under test here.
    centre.set_rules(NotificationRules {
        max_raised: 1_000,
        ..centre.rules()
    });

    // 1. `:ev <ns>`, then `ctrl-z`: only the faults of the test's namespace.
    let view = open_events_view(&mut vcx, &tab, ns.name());
    vcx.simulate_keystrokes("ctrl-z");
    wait(&mut vcx, "ctrl-z to show only the faults", |vcx| {
        vcx.update(|_, cx| view.read(cx).filters().faults_only)
    });

    let mut arrivals = EventArrivals::start(&context, ns.name(), DEADLINE).expect("the watch");
    create(&context, ns.name(), &unpullable_pod("oxi-ev-pull")).expect("the pod");
    let first = arrivals
        .wait_for(DEADLINE, |a| {
            a.is_warning_about("Pod", "oxi-ev-pull") && a.reason == PULL_FAILED_REASON
        })
        .unwrap_or_else(|| panic!("no pull failure\n{}\n{}", arrivals.describe(), ns.events()));
    let started = Instant::now();
    loop {
        vcx.run_until_parked();
        let shown = view_rows(&mut vcx, &view, "oxi-ev-pull");
        if shown.iter().any(|(_, _, uid)| *uid == first.uid) {
            break;
        }
        assert!(
            started.elapsed() < DEADLINE,
            "the view never showed the warning\n{}",
            view_text(&mut vcx, &view)
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let late = Instant::now().saturating_duration_since(first.at);
    eprintln!("events view: the Warning was shown {late:?} after kubectl saw it");
    assert!(
        late <= LIVE_WITHIN,
        "the view showed the Warning {late:?} after kubectl saw it\n{}",
        view_text(&mut vcx, &view)
    );
    let shown = view_rows(&mut vcx, &view, "oxi-ev-pull");
    assert!(
        shown.iter().all(|(t, _, _)| *t == EventType::Warning),
        "faults only: {shown:?}"
    );
    let outside = vcx.update(|_, cx| {
        view.read(cx)
            .model(cx)
            .events()
            .filter(|e| e.namespace.as_deref() != Some(ns.name()))
            .count()
    });
    assert_eq!(outside, 0, "only the test's namespace");

    // 2. The pods table's badge and the drawer's Events tab.
    let badge = wait_value(
        &mut vcx,
        "the pod's warning badge in the pods table",
        |vcx| badge_text(vcx, &table, ns.name(), "oxi-ev-pull").filter(|t| !t.is_empty()),
    );
    assert_eq!(badge, format!("Warning: {PULL_FAILED_REASON}"));
    let pod = vcx.update(|_, cx| {
        let table = table.read(cx);
        let ix = table.read_rows(cx, |rows| {
            rows.rows().iter().position(|r| {
                &*r.meta().name == "oxi-ev-pull" && r.meta().namespace.as_deref() == Some(ns.name())
            })
        });
        ix.and_then(|ix| table.row_ref(ix, cx))
    });
    let pod = pod.expect("the pod's row");
    dispatch(&mut vcx, Command::ResourceViewEvents { target: pod });
    let inner = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let detail = wait_value(&mut vcx, "the drawer to open on the pod", |vcx| {
        vcx.update(|_, cx| {
            inner
                .read(cx)
                .panel::<DetailDrawer>()
                .and_then(|d| d.read(cx).view().cloned())
        })
    });
    wait(&mut vcx, "the Events tab to list the warning", |vcx| {
        vcx.update(|_, cx| {
            let detail = detail.read(cx);
            detail.tab() == DetailTab::Events
                && detail
                    .event_rows()
                    .iter()
                    .any(|e| e.kind == EventType::Warning && &*e.reason == PULL_FAILED_REASON)
        })
    });

    // 3. A notification, raised as a toast.
    let row = wait_value(&mut vcx, "a notification about the pod", |_| {
        rows_about(&centre, ns.name(), "oxi-ev-pull")
            .into_iter()
            .next()
    });
    assert!(row.title.contains("oxi-ev-pull"), "{}", row.title);
    let ws = workspace(&mut vcx);
    let key = format!("notification/{}", row.id);
    wait(&mut vcx, "the notification's toast", |vcx| {
        vcx.update(|_, cx| {
            // The layer shows at most three notification toasts per ten seconds; past that, its
            // summary toast stands for the rest (other suites' pods may have taken the slots).
            ws.read(cx)
                .toast_layer()
                .read(cx)
                .visible()
                .iter()
                .any(|t| {
                    t.key.as_deref() == Some(key.as_str())
                        || t.key.as_deref() == Some(oxikube_workspace::notifications::SUMMARY_KEY)
                })
        })
    });

    // 4. Muted: the view still shows a new failing pod, the centre lists nothing about it.
    mute_then_unmute(&mut vcx, &view, &centre, &cluster, &context, &ns);
    drop((arrivals, dir));
}

fn mute_then_unmute(
    vcx: &mut VisualTestContext,
    view: &Entity<EventsView>,
    centre: &NotificationService,
    cluster: &ClusterId,
    context: &str,
    ns: &TestNamespace,
) {
    dispatch(
        vcx,
        Command::NotificationsMuteCluster {
            cluster: cluster.clone(),
        },
    );
    wait(vcx, "the cluster to be muted", |_| {
        centre.is_muted(cluster) && !centre.is_watching(cluster)
    });
    create(context, ns.name(), &unpullable_pod("oxi-ev-muted")).expect("the pod");
    wait(vcx, "the muted pod's warning in the view", |vcx| {
        !view_rows(vcx, view, "oxi-ev-muted").is_empty()
    });
    let quiet_until = Instant::now() + QUIET;
    while Instant::now() < quiet_until {
        vcx.run_until_parked();
        assert!(
            rows_about(centre, ns.name(), "oxi-ev-muted").is_empty(),
            "a muted cluster notified"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    dispatch(
        vcx,
        Command::NotificationsUnmuteCluster {
            cluster: cluster.clone(),
        },
    );
    wait(vcx, "the cluster to be watched again", |_| {
        !centre.is_muted(cluster) && centre.is_watching(cluster)
    });
    create(context, ns.name(), &unpullable_pod("oxi-ev-after")).expect("the pod");
    wait(
        vcx,
        "a notification about the pod created after unmuting",
        |_| !rows_about(centre, ns.name(), "oxi-ev-after").is_empty(),
    );
}
