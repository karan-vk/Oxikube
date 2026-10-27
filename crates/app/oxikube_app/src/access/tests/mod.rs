//! Access service tests against `FakeAccessReviewPort` and the virtual clock.

mod accessible;
mod batch;
mod cache;
mod registry;

use std::sync::Arc;
use std::time::Duration;

use oxikube_domain::access::{AccessCheck, AccessRule, AccessRules};
use oxikube_domain::ids::Gvr;
use oxikube_testkit::{FakeAccessReviewPort, FakeClockPort};

use super::{AccessService, DEFAULT_TTL};

fn service(
    port: FakeAccessReviewPort,
) -> (AccessService, Arc<FakeAccessReviewPort>, Arc<FakeClockPort>) {
    let port = Arc::new(port);
    let clock = Arc::new(FakeClockPort::default());
    let service = AccessService::new(port.clone(), clock.clone(), DEFAULT_TTL);
    (service, port, clock)
}

fn reads(resources: &[&str]) -> AccessRules {
    AccessRules::none().with_rule(AccessRule::granting(
        &["get", "list"],
        &[""],
        resources,
        &[],
    ))
}

fn pods() -> Gvr {
    Gvr::new("", "v1", "pods")
}

fn check(verb: &str, namespace: &str) -> AccessCheck {
    AccessCheck::new(verb, pods()).namespace(namespace)
}

const PAST_TTL: Duration = Duration::from_secs(61);
