//! The cache's keys and freshness rule.

use std::time::Duration;

use jiff::Timestamp;
use oxikube_ports::{CloudDetection, CloudProvider, CloudScope};
use proptest::prelude::*;

use super::aws_scope;
use crate::cloud::cache::{ProviderRow, ScopeRow, Snapshot, state_key};

const TTL: Duration = Duration::from_secs(600);

fn now() -> Timestamp {
    Timestamp::from_second(1_800_000_000).unwrap()
}

fn ms_ago(seconds: i64) -> i64 {
    now().as_millisecond() - seconds * 1000
}

fn snapshot(provider_age: i64, scopes: &[(CloudScope, i64)], listed: bool) -> Snapshot {
    Snapshot {
        provider: Some(ProviderRow {
            detection: CloudDetection::not_installed(CloudProvider::Aws),
            scopes: listed.then(|| scopes.iter().map(|(s, _)| s.clone()).collect()),
            fetched_at: ms_ago(provider_age),
        }),
        scopes: scopes
            .iter()
            .map(|(scope, age)| ScopeRow {
                scope: scope.clone(),
                fetched_at: ms_ago(*age),
                clusters: Vec::new(),
            })
            .collect(),
    }
}

#[test]
fn fresh_means_the_detection_and_every_scope_within_the_ttl() {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let both = [(prod.clone(), 10), (dev.clone(), 599)];
    assert!(snapshot(10, &both, true).is_fresh(None, TTL, now()));
    assert!(
        !snapshot(600, &both, true).is_fresh(None, TTL, now()),
        "old detection"
    );
    let old_dev = [(prod.clone(), 10), (dev, 601)];
    assert!(!snapshot(10, &old_dev, true).is_fresh(None, TTL, now()));
    assert!(
        snapshot(10, &old_dev, true).is_fresh(Some(std::slice::from_ref(&prod)), TTL, now()),
        "only the scopes asked for count"
    );
    assert!(
        !snapshot(10, &both, false).is_fresh(None, TTL, now()),
        "a scope list never reported is not fresh"
    );
    let future = [(prod, -60)];
    assert!(
        !snapshot(10, &future, true).is_fresh(None, TTL, now()),
        "a clock that went back is stale"
    );
    assert!(!Snapshot::default().is_fresh(None, TTL, now()));
    assert!(Snapshot::default().is_empty());
}

#[test]
fn keys_keep_plain_scopes_readable() {
    assert_eq!(
        state_key(&aws_scope("prod", "eu-west-1").key())
            .unwrap()
            .as_str(),
        "aws/prod/eu-west-1"
    );
    assert_eq!(
        state_key("aws/me@corp:x/y").unwrap().as_str(),
        "aws/me:40corp:3ax/y"
    );
}

proptest! {
    /// Two different scope keys never share a cache row.
    #[test]
    fn keys_are_injective(a in ".{1,20}", b in ".{1,20}") {
        prop_assume!(a != b);
        let (ka, kb) = (state_key(&a).unwrap(), state_key(&b).unwrap());
        prop_assert_ne!(ka.as_str(), kb.as_str());
    }

    #[test]
    fn every_profile_name_makes_a_valid_key(profile in ".{1,20}") {
        let scope = aws_scope(&profile, "eu-west-1");
        prop_assert!(state_key(&scope.key()).is_ok(), "{profile:?}");
    }
}
