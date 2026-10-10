//! `JumpRecents`: per cluster, deduplicated by text, capped, persisted.

use serde_json::json;

use super::*;
use crate::search::recents::{JUMP_CAPACITY, JUMP_TEXT_MAX_CHARS, JumpRecents};

fn history(state: &Arc<FakeStatePort>) -> JumpRecents {
    JumpRecents::new(state.clone())
}

fn key(cluster: &oxikube_domain::ids::ClusterId) -> String {
    format!("history.jump/{cluster}")
}

#[test]
fn the_latest_jump_comes_first_and_a_repeat_moves_up() {
    let state = fake();
    let history = history(&state);
    let prod = cluster("prod");
    for line in ["pods", "deploy kube-system", "ctx staging", "pods"] {
        assert!(history.record(&prod, line));
    }
    assert_eq!(
        history.recent(&prod),
        ["pods", "ctx staging", "deploy kube-system"]
    );
}

#[test]
fn history_is_deduplicated_by_the_text_not_the_spacing() {
    let state = fake();
    let history = history(&state);
    let prod = cluster("prod");
    history.record(&prod, "pod   app=nginx");
    history.record(&prod, "  pod app=nginx ");
    history.record(&prod, "pod\tapp=nginx\n");
    assert_eq!(history.recent(&prod), ["pod app=nginx"]);
    // Case is part of the text: `Pods` and `pods` are two lines.
    history.record(&prod, "Pods");
    history.record(&prod, "pods");
    assert_eq!(history.recent(&prod).len(), 3);
}

#[test]
fn each_cluster_has_its_own_history() {
    let state = fake();
    let history = history(&state);
    let (prod, staging) = (cluster("prod"), cluster("staging"));
    history.record(&prod, "deploy kube-system");
    history.record(&staging, "pods");
    assert_eq!(history.recent(&prod), ["deploy kube-system"]);
    assert_eq!(history.recent(&staging), ["pods"]);
    assert!(history.recent(&cluster("other")).is_empty());
    history.clear(&prod);
    assert!(history.recent(&prod).is_empty());
    assert_eq!(history.recent(&staging), ["pods"]);
}

#[test]
fn the_history_is_capped_keeping_the_latest() {
    let state = fake();
    let history = history(&state);
    let prod = cluster("prod");
    for i in 0..JUMP_CAPACITY + 30 {
        history.record(&prod, &format!("pod app=x{i}"));
    }
    let kept = history.recent(&prod);
    assert_eq!(kept.len(), JUMP_CAPACITY);
    assert_eq!(kept[0], format!("pod app=x{}", JUMP_CAPACITY + 29));
}

#[test]
fn history_survives_a_restart_per_cluster() {
    let state = fake();
    let first = history(&state);
    let (prod, staging) = (cluster("prod"), cluster("staging"));
    first.record(&prod, "deploy kube-system");
    first.record(&prod, "ctx staging");
    first.record(&staging, "pods");
    block_on(first.flush());
    assert_eq!(
        stored(&state, &key(&prod)),
        Some(json!({ "v": 1, "jumps": ["ctx staging", "deploy kube-system"] }))
    );
    assert_eq!(writes(&state).len(), 2, "one value per cluster");

    let second = history(&state);
    assert!(second.recent(&prod).is_empty());
    block_on(second.load(&prod));
    assert_eq!(second.recent(&prod), ["ctx staging", "deploy kube-system"]);
    assert!(second.recent(&staging).is_empty(), "loaded per cluster");
}

#[test]
fn load_reads_a_cluster_once_and_keeps_what_ran_since() {
    let state = fake();
    let old = history(&state);
    let prod = cluster("prod");
    old.record(&prod, "pods");
    old.record(&prod, "nodes");
    block_on(old.flush());

    let fresh = history(&state);
    fresh.record(&prod, "svc");
    fresh.record(&prod, "pods");
    state.clear_calls();
    block_on(fresh.load(&prod));
    block_on(fresh.load(&prod));
    assert_eq!(state.recorded_calls().len(), 1, "read once");
    assert_eq!(fresh.recent(&prod), ["pods", "svc", "nodes"]);
}

#[test]
fn only_the_changed_clusters_are_written() {
    let state = fake();
    let history = history(&state);
    let (prod, staging) = (cluster("prod"), cluster("staging"));
    history.record(&prod, "pods");
    history.record(&staging, "nodes");
    block_on(history.flush());
    state.clear_calls();
    history.record(&prod, "svc");
    block_on(history.flush());
    let written = writes(&state);
    assert_eq!(written.len(), 1);
    assert_eq!(written[0].0, key(&prod));
    block_on(history.flush());
    assert_eq!(writes(&state).len(), 1, "nothing changed, nothing written");
}

#[test]
fn blank_long_and_secret_looking_text_is_not_remembered() {
    let state = fake();
    let history = history(&state);
    let prod = cluster("prod");
    assert!(!history.record(&prod, ""));
    assert!(!history.record(&prod, "   \t\n"));
    assert!(!history.record(&prod, &"x".repeat(JUMP_TEXT_MAX_CHARS + 1)));
    assert!(history.record(&prod, &"x".repeat(JUMP_TEXT_MAX_CHARS)));
    for secret in [
        "Bearer abcdefghijklmnop1234567890",
        "token: sk_live_0123456789abcdef",
        "ctx https://user:hunter2@proxy.example.com",
        "password=hunter2",
    ] {
        assert!(!history.record(&prod, secret), "{secret}");
    }
    assert_eq!(history.recent(&prod).len(), 1);
    block_on(history.flush());
    let stored = stored(&state, &key(&prod)).unwrap().to_string();
    assert!(!stored.contains("hunter2") && !stored.contains("Bearer"));
}

#[test]
fn stored_lines_are_checked_again_on_load() {
    let state = fake();
    let prod = cluster("prod");
    block_on(oxikube_ports::StatePort::kv_set(
        &*state,
        &oxikube_ports::StateKey::new(key(&prod)).unwrap(),
        json!({ "v": 1, "jumps": ["pods", "password=hunter2", "", 5, "ctx  staging"] }),
    ))
    .unwrap();
    let history = history(&state);
    block_on(history.load(&prod));
    assert_eq!(history.recent(&prod), ["pods", "ctx staging"]);
}

#[test]
fn a_corrupt_value_loads_as_empty() {
    for bad in [
        json!("x"),
        json!({ "jumps": 3 }),
        json!(null),
        json!([1, 2]),
    ] {
        let state = fake();
        let prod = cluster("prod");
        block_on(oxikube_ports::StatePort::kv_set(
            &*state,
            &oxikube_ports::StateKey::new(key(&prod)).unwrap(),
            bad.clone(),
        ))
        .unwrap();
        let history = history(&state);
        block_on(history.load(&prod));
        assert!(history.recent(&prod).is_empty(), "{bad}");
        history.record(&prod, "pods");
        assert_eq!(history.recent(&prod), ["pods"]);
    }
}

#[test]
fn clearing_persists_an_empty_history() {
    let state = fake();
    let history = history(&state);
    let prod = cluster("prod");
    history.record(&prod, "pods");
    block_on(history.flush());
    history.clear(&prod);
    block_on(history.flush());
    assert_eq!(
        stored(&state, &key(&prod)),
        Some(json!({ "v": 1, "jumps": [] }))
    );
}
