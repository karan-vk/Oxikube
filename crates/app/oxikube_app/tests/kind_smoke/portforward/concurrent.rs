//! 50 concurrent forwards to one Service (epic E15 done-when item 5's load, without the UI): all
//! become `Active` on their own local ports and each answers a GET, all at once. Prints the time
//! to start them all and to serve one request through each, for the `--perf` baseline.

use std::time::Instant;

use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Service;
use oxikube_domain::portforward::ForwardPort;
use oxikube_testkit::integration::TestNamespace;
use oxikube_testkit::integration::nginx;

use super::{Forwarding, create, is_active, perf_line};
use crate::cluster::Kind;

const FORWARDS: usize = 50;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fifty_forwards_to_one_service() {
    let Some(kind) = Kind::from_env().await else {
        return;
    };
    let ns = TestNamespace::create(kind.context.as_str()).expect("namespace");
    let client = kind.admin_client().await;
    create::<Deployment>(&client, ns.name(), nginx::deployment("web", "web", 2)).await;
    create::<Service>(&client, ns.name(), nginx::service("web", "web")).await;
    crate::eventually("two ready pods", String::new, || async {
        super::ready_pods(&client, ns.name(), "web").await.len() == 2
    })
    .await;
    let forwarding = Forwarding::connect(&kind).await;
    let spec = forwarding.spec(
        "Service",
        ns.name(),
        "web",
        ForwardPort::Number(nginx::SERVICE_PORT),
    );

    let started = Instant::now();
    let ids: Vec<_> = (0..FORWARDS)
        .map(|_| forwarding.manager.start(spec.clone()).expect("starts"))
        .collect();
    crate::eventually(
        "every forward to be active",
        || {
            format!(
                "{:?}",
                forwarding
                    .manager
                    .list()
                    .iter()
                    .map(|f| &f.state)
                    .collect::<Vec<_>>()
            )
        },
        || {
            std::future::ready(
                forwarding
                    .manager
                    .list()
                    .iter()
                    .filter(|f| is_active(f))
                    .count()
                    == FORWARDS,
            )
        },
    )
    .await;
    let all_active = started.elapsed();
    let mut ports: Vec<_> = forwarding
        .manager
        .list()
        .iter()
        .map(|f| f.local_addr.expect("bound"))
        .collect();
    ports.sort();
    ports.dedup();
    assert_eq!(ports.len(), FORWARDS, "one local port each");

    let served = Instant::now();
    let pages = futures::future::join_all(
        ports
            .iter()
            .map(|addr| nginx::wait_welcome(*addr, crate::DEADLINE)),
    )
    .await;
    assert_eq!(pages.len(), FORWARDS);
    let all_served = served.elapsed();
    perf_line(
        "portforward.fifty_forwards",
        &[("all_active", all_active), ("all_served", all_served)],
    );
    for id in ids {
        assert!(forwarding.manager.remove(id));
    }
    assert!(forwarding.manager.list().is_empty());
}
