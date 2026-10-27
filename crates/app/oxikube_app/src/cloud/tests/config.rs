//! Settings-driven configuration (E18-S07): a changed config applies to the next run, a provider
//! switched off disappears at once.

use std::num::NonZeroUsize;
use std::time::Duration;

use oxikube_ports::CloudProvider;

use super::{Fixture, aws, aws_scope, eks, finished, names};
use crate::cloud::{DiscoveryConfig, RefreshMode};

fn disabled(provider: CloudProvider) -> DiscoveryConfig {
    DiscoveryConfig {
        disabled: [provider].into(),
        ..DiscoveryConfig::default()
    }
}

#[tokio::test]
async fn a_new_ttl_applies_to_the_next_refresh() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    f.advance(Duration::from_secs(120));

    // Within the default 15 minutes: answered from the cache.
    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(names(&events), ["cached-fresh", "finished"]);

    // A 30 s TTL (the settings changed): the same cache is now stale and the CLI runs again.
    f.service.set_config(DiscoveryConfig {
        ttl: Duration::from_secs(30),
        max_parallel_scopes: NonZeroUsize::new(2).unwrap(),
        ..DiscoveryConfig::default()
    });
    assert_eq!(f.service.config().max_parallel_scopes.get(), 2);
    let events = f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(names(&events)[0], "cached-stale");
    assert!(names(&events).contains(&"started"));
}

#[tokio::test]
async fn a_disabled_provider_is_hidden_and_does_not_run_and_enabling_brings_it_back() {
    let prod = aws_scope("prod", "eu-west-1");
    let port = aws([prod.clone()], [eks(&prod, "web")]);
    let f = Fixture::new(vec![port.clone()]);
    f.refresh(CloudProvider::Aws, RefreshMode::IfStale).await;
    assert_eq!(f.service.providers(), [CloudProvider::Aws]);

    f.service.set_config(disabled(CloudProvider::Aws));
    assert!(
        f.service.providers().is_empty(),
        "the card disappears at once"
    );
    assert!(f.service.cached(CloudProvider::Aws).await.is_empty());
    let calls = port.recorded_calls().len();
    let events = f.refresh(CloudProvider::Aws, RefreshMode::Force).await;
    assert_eq!(names(&events), ["finished"], "no detection, no listing");
    assert!(finished(&events).0.is_empty());
    assert_eq!(
        port.recorded_calls().len(),
        calls,
        "the port was not called"
    );

    f.service.set_config(DiscoveryConfig::default());
    assert_eq!(f.service.providers(), [CloudProvider::Aws]);
    assert_eq!(f.service.cached(CloudProvider::Aws).await.len(), 1);
}
