//! `events::SendToAgent`: the selected events become a queued context block.

use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};

use super::*;
use crate::command_bus::{CommandOutput, DispatchContext, Outcome};
use crate::context::CollectingConsumer;

fn web() -> ResourceRef {
    ResourceRef::namespaced(cluster(), Gvk::new("", "v1", "Pod"), "shop", "web-0")
}

fn fixture() -> Vec<Event> {
    vec![
        ev("e1", "Pod", "shop", "web-0", "uid-web", 10),
        warning("e2", "Pod", "shop", "web-0", "uid-web", 30, "BackOff"),
        warning("e3", "Pod", "shop", "db-0", "uid-db", 20, "Unhealthy"),
    ]
}

async fn run(h: &Harness, command: Command) -> CommandOutput {
    match h
        .bus()
        .dispatch(command, DispatchContext::new(Initiator::Ui, "me"))
        .await
    {
        Ok(Outcome::Completed(output)) => output,
        other => panic!("expected a completed command, got {other:?}"),
    }
}

#[tokio::test]
async fn an_objects_events_are_queued_with_where_they_came_from() {
    let h = Harness::with_events(fixture()).await;
    let output = run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: Some(web()),
            events: Vec::new(),
        },
    )
    .await;
    assert!(
        output
            .message
            .unwrap()
            .starts_with("Queued 2 events for the agent (1 waiting)")
    );
    assert_eq!(output.data.as_ref().unwrap()["delivery"], "queued");
    assert_eq!(h.pending.pending(), 1);

    // The agent panel attaches and receives it.
    let consumer = CollectingConsumer::new();
    let _guard = h.pending.attach(consumer.clone());
    let items = consumer.take();
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.block.title, "Events: events of Pod/shop/web-0");
    assert!(item.block.body.contains("BackOff"), "{}", item.block.body);
    assert!(!item.block.body.contains("db-0"), "{}", item.block.body);
    assert_eq!(item.source.cluster, cluster());
    assert_eq!(item.source.cluster_name, "dev");
    assert_eq!(item.source.namespace, "shop");
    assert_eq!(item.source.subject, "events of Pod/shop/web-0");
    assert_eq!(item.source.lines, 2);
    assert_eq!(
        item.source.span,
        Some((
            t0() + jiff::SignedDuration::from_secs(10),
            t0() + jiff::SignedDuration::from_secs(30)
        ))
    );
}

#[tokio::test]
async fn selected_events_are_sent_by_uid() {
    let h = Harness::with_events(fixture()).await;
    let consumer = CollectingConsumer::new();
    let _guard = h.pending.attach(consumer.clone());
    let output = run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: Some(web()),
            events: vec!["e3".into(), "gone".into()],
        },
    )
    .await;
    assert_eq!(output.message.as_deref(), Some("Sent 1 event to the agent"));
    assert_eq!(output.data.unwrap()["delivery"], "delivered");
    let items = consumer.take();
    assert_eq!(items[0].source.subject, "1 selected event");
    assert!(
        items[0].block.body.contains("Unhealthy"),
        "{}",
        items[0].block.body
    );
    assert!(
        !items[0].block.body.contains("BackOff"),
        "uids win over the target"
    );
}

#[tokio::test]
async fn with_no_selection_the_newest_warnings_go() {
    let h = Harness::with_events(fixture()).await;
    let consumer = CollectingConsumer::new();
    let _guard = h.pending.attach(consumer.clone());
    run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: None,
            events: Vec::new(),
        },
    )
    .await;
    let items = consumer.take();
    assert_eq!(items[0].source.subject, "the newest warning events");
    assert_eq!(items[0].source.lines, 2);
    assert!(
        !items[0].block.body.contains("Normal"),
        "{}",
        items[0].block.body
    );
}

#[tokio::test]
async fn nothing_to_send_queues_nothing_and_says_so() {
    let h = Harness::with_events([ev("e1", "Pod", "shop", "web-0", "uid-web", 10)]).await;
    let output = run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: None,
            events: Vec::new(),
        },
    )
    .await;
    assert_eq!(
        output.message.as_deref(),
        Some("There are no warning events to send.")
    );
    assert_eq!(output.data.unwrap()["sent"], 0);
    assert_eq!(h.pending.pending(), 0);
}

#[tokio::test]
async fn a_target_in_another_cluster_is_refused() {
    let h = Harness::with_events(fixture()).await;
    let other = ClusterId::new("/home/me/.kube/config", &ContextName::new("prod"));
    let bad = ResourceRef::namespaced(other, Gvk::new("", "v1", "Pod"), "shop", "web-0");
    let error = h
        .bus()
        .dispatch(
            Command::EventsSendToAgent {
                cluster: cluster(),
                target: Some(bad),
                events: Vec::new(),
            },
            DispatchContext::new(Initiator::Agent, "agent"),
        )
        .await
        .expect_err("refused");
    assert!(error.to_string().contains("another cluster"), "{error}");
}

#[tokio::test]
async fn it_runs_on_a_read_only_cluster() {
    let h = Harness::with_events(fixture()).await;
    h.sessions
        .set_prefs_table(oxikube_ports::ClusterPrefsTable::new(
            oxikube_ports::ClusterPrefs {
                read_only: true,
                ..oxikube_ports::ClusterPrefs::default()
            },
        ));
    let output = run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: Some(web()),
            events: Vec::new(),
        },
    )
    .await;
    assert_eq!(output.data.unwrap()["sent"], 2);
}

#[tokio::test]
async fn a_full_queue_drops_the_oldest_item_not_the_send() {
    let h = Harness::with_events(fixture()).await;
    for _ in 0..crate::context::MAX_PENDING_ITEMS + 1 {
        run(
            &h,
            Command::EventsSendToAgent {
                cluster: cluster(),
                target: None,
                events: Vec::new(),
            },
        )
        .await;
    }
    assert_eq!(h.pending.pending(), crate::context::MAX_PENDING_ITEMS);
    assert_eq!(h.pending.dropped(), 1);
}

#[tokio::test]
async fn a_pod_of_the_same_name_in_another_namespace_is_not_mixed_in() {
    let h = Harness::with_events([
        warning("a", "Pod", "shop", "web-0", "uid-a", 10, "BackOff"),
        warning("b", "Pod", "staging", "web-0", "uid-b", 20, "Unhealthy"),
    ])
    .await;
    let consumer = CollectingConsumer::new();
    let _guard = h.pending.attach(consumer.clone());
    run(
        &h,
        Command::EventsSendToAgent {
            cluster: cluster(),
            target: Some(web()),
            events: Vec::new(),
        },
    )
    .await;
    let items = consumer.take();
    assert_eq!(items[0].source.lines, 1);
    assert!(
        items[0].block.body.contains("BackOff"),
        "{}",
        items[0].block.body
    );
    assert!(
        !items[0].block.body.contains("Unhealthy"),
        "{}",
        items[0].block.body
    );
}
