//! Scenario 3: a Deployment whose pods cannot pull their image is a burst of `Failed` Warnings
//! over several pods. The `NotificationService` (default rules: every watched kind, the group
//! threshold, the raise limit) lists one row per pod up to the threshold and folds the rest of
//! the burst into exactly one group row; each row is raised (a toast, the OS) once, however many
//! times the kubelet repeats its warning. Muting the cluster holds every new warning back (a new
//! failing pod is in the `EventService`, not in the centre) and is saved; unmuting resumes with
//! what happens next.
//!
//! The cluster's feed is narrowed to the test's namespace (`ScopedEvents`), so the counts are
//! this test's alone.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use oxikube_app::events::EventFilter;
use oxikube_app::notifications::{
    MUTED_KEY, Notification, NotificationId, NotificationRules, NotificationService,
};
use oxikube_domain::event::Event;
use oxikube_ports::{StateKey, StatePort as _};
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::events::{
    PULL_FAILED_REASON, unpullable_deployment, unpullable_pod,
};
use oxikube_testkit::{EventsConnector, FakeStatePort, ScopedEvents};
use parking_lot::Mutex;

use super::{Chain, create_in, kube_connector, tokio_spawner, until};
use crate::clock::TokioClock;
use crate::cluster::Kind;

/// Pods in the burst: two more than the group threshold, so the group row stands for two.
const EXTRA: usize = 2;

/// How long a muted cluster is given to (not) notify about a warning the service already holds.
/// The pump turns an event into a notification in milliseconds.
const QUIET: Duration = Duration::from_secs(3);

/// The pull-failure warnings the service holds about pods in `ns`, by pod name.
fn failing_pods(chain: &Chain, ns: &TestNamespace) -> BTreeSet<String> {
    chain
        .events
        .query(
            &chain.cluster,
            &EventFilter::warnings()
                .namespaces([ns.name()])
                .kinds(["Pod"])
                .reasons([PULL_FAILED_REASON]),
        )
        .iter()
        .map(|e: &Arc<Event>| e.regarding.name.to_string())
        .collect()
}

/// The rows about the pod `name` (own rows; a group row has no single target).
fn rows_about(centre: &NotificationService, name: &str) -> Vec<Arc<Notification>> {
    centre
        .list()
        .into_iter()
        .filter(|row| !row.grouped && row.target.as_ref().is_some_and(|t| &*t.name == name))
        .collect()
}

fn describe(rows: &[Arc<Notification>]) -> String {
    rows.iter()
        .map(|r| {
            format!(
                "#{} {} (x{}, {} objects, grouped {})",
                r.id, r.title, r.count, r.objects, r.grouped
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_notifies_once_per_pod_and_once_for_the_rest_and_mute_holds_it_back() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let scope = ns.name().to_owned();
    let chain = Chain::connect(
        &kind,
        Arc::new(EventsConnector::new(
            Arc::new(kube_connector(&kind)),
            move |_, events| Arc::new(ScopedEvents::new(events).namespaces([scope.as_str()])),
        )),
    )
    .await;
    let state = Arc::new(FakeStatePort::new());
    let centre = NotificationService::new(
        chain.events.clone(),
        chain.sessions.clone(),
        state.clone(),
        Arc::new(TokioClock),
    );
    centre.start(tokio_spawner());
    let cluster = chain.cluster.clone();
    until(
        "the centre to watch the cluster",
        || chain.diagnostics(&ns),
        || centre.is_watching(&cluster),
    )
    .await;
    chain.ready(&ns).await;

    // Every raise, per row: a toast and the OS see each of these.
    let raised: Arc<Mutex<HashMap<NotificationId, usize>>> = Arc::default();
    let sink = raised.clone();
    let mut changes = centre.subscribe();
    let counter = tokio::spawn(async move {
        while let Some(change) = changes.next().await {
            let mut raised = sink.lock();
            for row in &change.raised {
                *raised.entry(row.id).or_default() += 1;
            }
        }
    });

    let rules = centre.rules();
    let replicas = rules.group_threshold + EXTRA;
    let deployment = unpullable_deployment("burst", u32::try_from(replicas).expect("small"));
    create_in(&kind, &ns, &deployment);
    let diagnostics = || {
        format!(
            "{}\ncentre:\n{}",
            chain.diagnostics(&ns),
            describe(&centre.list())
        )
    };
    until(
        "every pod of the burst to fail its pull",
        diagnostics,
        || failing_pods(&chain, &ns).len() == replicas,
    )
    .await;
    // The first warning of each pod lands; the kubelet repeats them (ErrImagePull, then
    // ImagePullBackOff) for as long as the test runs.
    until("the burst to be listed", diagnostics, || {
        let rows = centre.list();
        rows.iter().filter(|r| !r.grouped).count() == rules.group_threshold
            && rows
                .iter()
                .any(|r| r.grouped && r.objects as usize == EXTRA)
    })
    .await;
    tokio::time::sleep(QUIET).await;
    let rows = centre.list();
    let own: Vec<_> = rows.iter().filter(|r| !r.grouped).collect();
    let groups: Vec<_> = rows.iter().filter(|r| r.grouped).collect();
    assert_eq!(own.len(), rules.group_threshold, "{}", describe(&rows));
    assert_eq!(
        groups.len(),
        1,
        "the rest of the burst is one row: {}",
        describe(&rows)
    );
    assert_eq!(groups[0].objects as usize, EXTRA, "{}", describe(&rows));
    assert_eq!(
        &*groups[0].title,
        format!("{EXTRA} pods: {PULL_FAILED_REASON}")
    );
    let pods: BTreeSet<_> = own
        .iter()
        .map(|r| r.target.as_ref().expect("a pod").name.to_string())
        .collect();
    assert_eq!(
        pods.len(),
        own.len(),
        "one row per pod: {}",
        describe(&rows)
    );
    assert_eq!(rows.len(), rules.group_threshold + 1);
    {
        let raised = raised.lock();
        for row in &rows {
            assert_eq!(
                raised.get(&row.id),
                Some(&1),
                "row #{} raised once however often its pod warned: {raised:?}",
                row.id
            );
        }
    }

    // Muted: nothing new is listed or raised, and the choice is saved.
    assert!(centre.mute(&cluster).await.expect("saved"));
    assert!(centre.is_muted(&cluster) && !centre.is_watching(&cluster));
    let saved = state
        .kv_get(&StateKey::new(MUTED_KEY).expect("a key"))
        .await
        .expect("readable")
        .expect("the muted clusters are saved");
    assert!(
        saved["clusters"]
            .as_array()
            .is_some_and(|ids| ids.contains(&serde_json::to_value(&cluster).expect("an id"))),
        "{saved}"
    );
    let before = (centre.list(), raised.lock().values().sum::<usize>());
    create_in(&kind, &ns, &unpullable_pod("muted"));
    until(
        "the muted pod's warning in the service",
        diagnostics,
        || failing_pods(&chain, &ns).contains("muted"),
    )
    .await;
    let quiet_until = Instant::now() + QUIET;
    while Instant::now() < quiet_until {
        assert!(
            rows_about(&centre, "muted").is_empty(),
            "a muted cluster notified"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(centre.list(), before.0, "nothing changed while muted");
    assert_eq!(
        raised.lock().values().sum::<usize>(),
        before.1,
        "nothing raised while muted"
    );

    // Unmuted: what happens next notifies again (the raise limit is not this step's subject, so
    // the burst's own raises do not count against the new pod).
    centre.set_rules(NotificationRules {
        max_raised: 100,
        ..rules
    });
    assert!(centre.unmute(&cluster).await.expect("saved"));
    until("the centre to watch the cluster again", diagnostics, || {
        centre.is_watching(&cluster)
    })
    .await;
    create_in(&kind, &ns, &unpullable_pod("after"));
    until(
        "a row about the pod created after unmuting",
        diagnostics,
        || !rows_about(&centre, "after").is_empty(),
    )
    .await;
    let row = rows_about(&centre, "after").remove(0);
    until("that row to be raised", diagnostics, || {
        raised.lock().get(&row.id) == Some(&1)
    })
    .await;
    counter.abort();
}
