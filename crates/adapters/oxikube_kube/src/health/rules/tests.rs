use std::sync::atomic::{AtomicUsize, Ordering};

use oxikube_domain::OxiError;

use super::*;

fn ctx(name: &str) -> ContextName {
    ContextName::new(name)
}

fn snapshot(verb: &str) -> RulesSnapshot {
    RulesSnapshot {
        rules: vec![AccessRule {
            verbs: vec![verb.into()],
            api_groups: vec!["*".into()],
            resources: vec!["*".into()],
            resource_names: vec![],
        }],
        ..Default::default()
    }
}

#[tokio::test(start_paused = true)]
async fn second_lookup_within_the_ttl_is_served_from_cache() {
    let cache = RulesCache::new(Duration::from_secs(30));
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(snapshot("get"))
    };
    let a = cache
        .get_or_fetch(&ctx("c"), "default", fetch)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(29)).await;
    let b = cache
        .get_or_fetch(&ctx("c"), "default", fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&a, &b));
}

#[tokio::test(start_paused = true)]
async fn entry_expires_after_the_ttl() {
    let cache = RulesCache::new(Duration::from_secs(30));
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(snapshot("get"))
    };
    cache
        .get_or_fetch(&ctx("c"), "default", fetch)
        .await
        .unwrap();
    tokio::time::advance(Duration::from_secs(30)).await;
    assert!(cache.get(&ctx("c"), "default").is_none());
    cache
        .get_or_fetch(&ctx("c"), "default", fetch)
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(start_paused = true)]
async fn namespaces_and_contexts_are_cached_separately() {
    let cache = RulesCache::new(Duration::from_secs(30));
    let calls = AtomicUsize::new(0);
    let fetch = || async {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(snapshot("get"))
    };
    for (c, ns) in [("a", "x"), ("a", "y"), ("b", "x")] {
        cache.get_or_fetch(&ctx(c), ns, fetch).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    for (c, ns) in [("a", "x"), ("a", "y"), ("b", "x")] {
        cache.get_or_fetch(&ctx(c), ns, fetch).await.unwrap();
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3, "all three now hit");
}

#[tokio::test(start_paused = true)]
async fn failures_are_not_cached() {
    let cache = RulesCache::default();
    let err = cache
        .get_or_fetch(&ctx("c"), "default", || async {
            Err(OxiError::network("down"))
        })
        .await
        .unwrap_err();
    assert!(err.is_retryable());
    let ok = cache
        .get_or_fetch(&ctx("c"), "default", || async { Ok(snapshot("get")) })
        .await;
    assert!(ok.is_ok());
}

#[tokio::test(start_paused = true)]
async fn invalidate_context_drops_only_that_context() {
    let cache = RulesCache::default();
    for c in ["a", "b"] {
        cache
            .get_or_fetch(&ctx(c), "ns", || async { Ok(snapshot("get")) })
            .await
            .unwrap();
    }
    cache.invalidate_context(&ctx("a"));
    assert!(cache.get(&ctx("a"), "ns").is_none());
    assert!(cache.get(&ctx("b"), "ns").is_some());
    cache.clear();
    assert!(cache.get(&ctx("b"), "ns").is_none());
}

#[test]
fn status_conversion_keeps_rules_and_partiality() {
    let status = SubjectRulesReviewStatus {
        resource_rules: vec![ResourceRule {
            api_groups: Some(vec!["".into()]),
            resources: Some(vec!["pods".into()]),
            resource_names: None,
            verbs: vec!["get".into()],
        }],
        non_resource_rules: vec![],
        incomplete: true,
        evaluation_error: Some("boom".into()),
    };
    let snap = RulesSnapshot::from_status(Some(&status));
    assert_eq!(snap.rules.len(), 1);
    assert_eq!(snap.rules[0].api_groups, vec![String::new()]);
    assert!(snap.rules[0].resource_names.is_empty());
    assert!(snap.incomplete && snap.is_partial());
    assert_eq!(snap.evaluation_error.as_deref(), Some("boom"));
}

#[test]
fn missing_status_is_unknown_not_denied() {
    let snap = RulesSnapshot::from_status(None);
    assert!(snap.is_partial());
}
