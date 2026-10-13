//! `@events`: an object's events as a block, sorted, bounded, with absence and errors visible.

use oxikube_domain::ErrorKind;
use oxikube_domain::agent::ContextBlock;
use oxikube_ports::{ContextScope, Mention};

use super::*;

async fn resolve(h: &Harness, mention: &str, scope: &ContextScope) -> Vec<ContextBlock> {
    h.contexts
        .resolve(&Mention::parse(mention).expect("a mention"), scope)
        .await
        .expect("resolved")
}

fn scope() -> ContextScope {
    ContextScope::new().with_cluster(cluster())
}

fn web_events() -> Vec<Event> {
    vec![
        ev("e1", "Pod", "shop", "web-0", "uid-web", 10),
        warning("e2", "Pod", "shop", "web-0", "uid-web", 30, "BackOff"),
        ev("e3", "Pod", "shop", "db-0", "uid-db", 20),
    ]
}

#[tokio::test]
async fn a_mention_resolves_to_the_objects_events_newest_first() {
    let h = Harness::with_events(web_events()).await;
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let blocks = resolve(&h, "@events/pod/shop/web-0", &scope()).await;
    assert_eq!(blocks.len(), 1);
    let block = &blocks[0];
    assert_eq!(block.title, "Events Pod/shop/web-0");
    let body = &block.body;
    assert!(body.contains("# cluster: dev"), "{body}");
    assert!(body.contains("# object: Pod/shop/web-0"), "{body}");
    assert!(body.contains("# events: 2 held, 2 quoted"), "{body}");
    let backoff = body
        .find("Warning BackOff x1 Pod/shop/web-0")
        .expect("the warning");
    let pulled = body
        .find("Normal Pulled x1 Pod/shop/web-0")
        .expect("the normal one");
    assert!(backoff < pulled, "newest first by last seen:\n{body}");
    assert!(
        !body.contains("db-0"),
        "another object's events stay out:\n{body}"
    );
    assert!(!block.truncated);
}

#[tokio::test]
async fn a_kind_word_and_a_cluster_scoped_object_resolve_too() {
    let mut node = ev("n1", "Node", "default", "worker-1", "uid-node", 5);
    node.regarding.namespace = None;
    node.reason = "NodeNotReady".into();
    let h = Harness::with_events([web_events(), vec![node]].concat()).await;
    h.objects().insert(
        Resource::from_json(serde_json::json!({
            "apiVersion": "v1", "kind": "Node",
            "metadata": {"name": "worker-1", "uid": "uid-node"},
        }))
        .expect("a node"),
    );
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let blocks = resolve(&h, "@events/node/worker-1", &scope()).await;
    assert!(
        blocks[0].body.contains("NodeNotReady"),
        "{}",
        blocks[0].body
    );
    let blocks = resolve(&h, "@events/pods/shop/web-0", &scope()).await;
    assert!(
        blocks[0].body.contains("BackOff"),
        "plural words work: {}",
        blocks[0].body
    );
}

#[tokio::test]
async fn an_object_that_does_not_exist_is_a_clear_error_block() {
    let h = Harness::with_events(web_events()).await;
    let blocks = resolve(&h, "@events/pod/shop/ghost", &scope()).await;
    let body = &blocks[0].body;
    assert!(
        body.contains("# error: Pod/shop/ghost was not found"),
        "{body}"
    );
    assert!(body.contains("no events about it are held"), "{body}");
}

#[tokio::test]
async fn a_gone_object_with_events_is_quoted_with_a_note() {
    let h = Harness::with_events(web_events()).await;
    // No pod in the cluster any more, but its events are still held.
    let blocks = resolve(&h, "@events/pod/shop/web-0", &scope()).await;
    let body = &blocks[0].body;
    assert!(
        body.contains("# note: Pod/shop/web-0 no longer exists"),
        "{body}"
    );
    assert!(body.contains("BackOff"), "{body}");
}

#[tokio::test]
async fn an_unknown_kind_is_a_clear_error_block_with_suggestions() {
    let h = Harness::with_events(web_events()).await;
    let blocks = resolve(&h, "@events/podz/shop/web-0", &scope()).await;
    let body = &blocks[0].body;
    assert!(body.contains("# error: unknown kind \"podz\""), "{body}");
}

#[tokio::test]
async fn an_object_without_events_says_none_are_held() {
    let h = Harness::with_events([]).await;
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let blocks = resolve(&h, "@events/pod/shop/web-0", &scope()).await;
    let body = &blocks[0].body;
    assert!(body.contains("# events: 0 held, 0 quoted"), "{body}");
    assert!(
        body.contains("# no events are held for this selection"),
        "{body}"
    );
    assert!(!body.contains("# error"), "{body}");
}

#[tokio::test]
async fn a_namespace_mention_quotes_its_newest_events() {
    let h = Harness::with_events(
        [
            web_events(),
            vec![ev("k1", "Pod", "kube-system", "dns-0", "uid-dns", 40)],
        ]
        .concat(),
    )
    .await;
    let blocks = resolve(&h, "@events/shop", &scope()).await;
    let body = &blocks[0].body;
    assert_eq!(blocks[0].title, "Events in namespace shop");
    assert!(body.contains("# events: 3 held, 3 quoted"), "{body}");
    assert!(!body.contains("dns-0"), "{body}");
    // The thread's namespace fills a bare `@events`.
    let bare = resolve(
        &h,
        "@events",
        &scope().with_default_namespace("kube-system"),
    )
    .await;
    assert!(bare[0].body.contains("dns-0"), "{}", bare[0].body);
}

#[tokio::test]
async fn a_mention_of_the_wrong_shape_is_a_validation_error() {
    let h = Harness::with_events([]).await;
    let error = h
        .contexts
        .resolve(&Mention::parse("@events/pod/a/b/c").unwrap(), &scope())
        .await
        .expect_err("too many segments");
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(
        error
            .message()
            .contains("@events/<kind>/<namespace>/<name>")
    );
}

#[tokio::test]
async fn a_cluster_that_is_not_connected_is_an_error() {
    let h = Harness::new();
    h.connect().await;
    h.sessions.disconnect(&cluster()).expect("disconnect");
    let error = h
        .contexts
        .resolve(&Mention::parse("@events/pod/shop/web-0").unwrap(), &scope())
        .await
        .expect_err("not connected");
    assert_eq!(error.kind(), ErrorKind::Network);
}

#[tokio::test]
async fn the_size_budget_drops_the_oldest_events_deterministically() {
    let many: Vec<Event> = (0..60)
        .map(|i| ev(&format!("e{i:02}"), "Pod", "shop", "web-0", "uid-web", i))
        .collect();
    let h = Harness::with_events(many).await;
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let small = scope().with_max_total_bytes(2_400);
    let first = resolve(&h, "@events/pod/shop/web-0", &small).await;
    let second = resolve(&h, "@events/pod/shop/web-0", &small).await;
    assert_eq!(
        first, second,
        "the same events in the same budget give the same block"
    );
    let block = &first[0];
    assert!(block.body.len() <= 2_400, "{} bytes", block.body.len());
    assert!(block.truncated);
    assert!(
        block.body.contains("older events left out"),
        "{}",
        block.body
    );
    // The newest events stay, the oldest go.
    assert!(block.body.contains("message of e59"), "{}", block.body);
    assert!(!block.body.contains("message of e00"), "{}", block.body);
}

#[tokio::test]
async fn secrets_in_messages_are_masked() {
    let mut leaky = ev("e1", "Pod", "shop", "web-0", "uid-web", 10);
    leaky.message = "Failed to pull \"https://admin:hunter2@registry.example.com/web:1\"".into();
    let h = Harness::with_events([leaky]).await;
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let blocks = resolve(&h, "@events/pod/shop/web-0", &scope()).await;
    assert!(!blocks[0].body.contains("hunter2"), "{}", blocks[0].body);
}

#[tokio::test]
async fn a_failed_feed_gives_a_block_that_says_why() {
    let h = Harness::new();
    h.connect().await;
    h.port()
        .script()
        .watch_events
        .push_err(oxikube_domain::OxiError::forbidden("events is forbidden"));
    h.objects().insert(pod_object("shop", "web-0", "uid-web"));
    let blocks = resolve(&h, "@events/pod/shop/web-0", &scope()).await;
    let body = &blocks[0].body;
    assert!(body.contains("# note: events are unavailable"), "{body}");
    assert!(body.contains("# feed: failed"), "{body}");
}
