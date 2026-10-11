//! Diff vs live (E10-S07), pure: the diff ignores comments, key order and the server's fields;
//! the changed-on-server state machine reloads clean buffers, raises the banner over dirty ones,
//! and each of Merge / Reload / Keep mine ends where the story says.

use std::sync::Arc;

use oxikube_domain::Resource;
use oxikube_editor::diff::{
    LiveObject, LiveSync, ManifestDiff, MergeSide, MergeStep, ObjectVersion, SyncAction, SyncState,
    default_renderer, document_for,
};
use serde_json::{Value, json};

fn deployment(replicas: i64, image: &str, rv: &str) -> Value {
    json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": "web", "namespace": "shop", "resourceVersion": rv, "uid": "1234",
            "generation": 3, "creationTimestamp": "2026-10-01T00:00:00Z",
            "managedFields": [{"manager": "kubectl", "operation": "Apply"}]
        },
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {"app": "web"}},
            "template": {
                "metadata": {"labels": {"app": "web"}},
                "spec": {"containers": [{"name": "web", "image": image}]}
            }
        },
        "status": {"replicas": replicas, "readyReplicas": 1}
    })
}

const BUFFER: &str = "\
# the shop's front end
apiVersion: apps/v1
kind: Deployment
metadata:
  namespace: shop
  name: web   # keep the name
spec:
  replicas: 3
  template:
    spec:
      containers:
        - image: nginx:1.27
          name: web
    metadata:
      labels: {app: web}
  selector:
    matchLabels:
      app: web
";

fn diff(live: &Value, buffer: &str) -> ManifestDiff {
    let mine = document_for(buffer, None).expect("the buffer parses");
    ManifestDiff::of_manifests(Some(live), &mine)
}

#[test]
fn comments_key_order_and_server_fields_give_an_empty_diff() {
    let live = deployment(3, "nginx:1.27", "41");
    let diff = diff(&live, BUFFER);
    assert!(diff.is_empty(), "{:?}", diff.inline_rows());
}

#[test]
fn a_comment_only_change_gives_an_empty_diff() {
    let live = deployment(3, "nginx:1.27", "41");
    let edited = BUFFER.replace("# keep the name", "# a new comment");
    assert!(diff(&live, &edited).is_empty());
}

#[test]
fn a_replica_change_is_one_hunk() {
    let live = deployment(3, "nginx:1.27", "41");
    let edited = BUFFER.replace("replicas: 3", "replicas: 5");
    let diff = diff(&live, &edited);
    assert_eq!(diff.hunks(), 1);
    assert_eq!(diff.counts(), (1, 1));
    let lines: Vec<_> = diff
        .inline_rows()
        .iter()
        .filter_map(|row| match row {
            oxikube_editor::diff::InlineRow::Line {
                kind: oxikube_editor::diff::LineKind::Added,
                new: Some(n),
                ..
            } => Some(diff.new_line(*n).trim().to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(lines, ["replicas: 5"]);
}

#[test]
fn managed_fields_resource_version_and_status_changes_give_an_empty_diff() {
    let before = deployment(3, "nginx:1.27", "41");
    let mut after = deployment(3, "nginx:1.27", "97");
    after["status"]["readyReplicas"] = json!(3);
    after["metadata"]["managedFields"] = json!([{"manager": "kube-controller-manager"}]);
    let diff = ManifestDiff::of_manifests(Some(&before), &after);
    assert!(diff.is_empty(), "{:?}", diff.inline_rows());
}

#[test]
fn a_missing_object_shows_every_line_added() {
    let mine = document_for(BUFFER, None).unwrap();
    let diff = ManifestDiff::of_manifests(None, &mine);
    let (removed, added) = diff.counts();
    assert_eq!(removed, 0);
    assert!(added > 5);
}

fn version(value: Value) -> Arc<ObjectVersion> {
    let resource = Resource::from_json(value).unwrap();
    Arc::new(ObjectVersion::new(resource, &default_renderer()))
}

fn present(value: Value) -> LiveObject {
    LiveObject::Present(version(value))
}

/// A sync loaded from the Deployment at 3 replicas.
fn loaded() -> (LiveSync, Arc<ObjectVersion>) {
    let base = version(deployment(3, "nginx:1.27", "41"));
    let mut sync = LiveSync::new();
    sync.loaded(base.clone());
    (sync, base)
}

#[test]
fn a_clean_buffer_reloads_silently() {
    let (mut sync, _) = loaded();
    let action = sync.live_changed(present(deployment(4, "nginx:1.27", "42")), false);
    let SyncAction::Reload(text) = action else {
        panic!("{action:?}")
    };
    assert!(text.contains("replicas: 4"));
    assert_eq!(sync.state(), SyncState::Clean);
    assert_eq!(sync.live_resource_version(), Some("42"));
}

#[test]
fn a_dirty_buffer_raises_the_banner_but_not_for_status_only() {
    let (mut sync, _) = loaded();
    sync.edited(true);
    assert_eq!(sync.state(), SyncState::Dirty);
    let mut status_only = deployment(3, "nginx:1.27", "42");
    status_only["status"]["readyReplicas"] = json!(3);
    assert_eq!(
        sync.live_changed(present(status_only), true),
        SyncAction::None
    );
    assert_eq!(sync.state(), SyncState::Dirty);
    assert_eq!(
        sync.live_changed(present(deployment(4, "nginx:1.27", "43")), true),
        SyncAction::Banner
    );
    assert_eq!(sync.state(), SyncState::Banner);
}

#[test]
fn reload_from_the_banner_ends_clean_with_the_server_text() {
    let (mut sync, _) = loaded();
    sync.edited(true);
    sync.live_changed(present(deployment(4, "nginx:1.27", "43")), true);
    let text = sync.reload().expect("the live object");
    assert!(text.contains("replicas: 4"));
    assert_eq!(sync.state(), SyncState::Clean);
}

#[test]
fn keep_mine_ends_dirty_and_the_same_change_does_not_ask_again() {
    let (mut sync, _) = loaded();
    sync.edited(true);
    let server = deployment(4, "nginx:1.27", "43");
    sync.live_changed(present(server.clone()), true);
    sync.keep_mine();
    assert_eq!(sync.state(), SyncState::Dirty);
    assert_eq!(
        sync.live_changed(present(server), true),
        SyncAction::None,
        "the acknowledged version is the base now"
    );
}

#[test]
fn keep_mine_without_a_banner_leaves_a_clean_buffer_reloading_silently() {
    // `editor::KeepMine` from the palette or the MCP tool, with no banner up.
    let (mut sync, _) = loaded();
    sync.keep_mine();
    assert_eq!(sync.state(), SyncState::Clean, "nothing to answer");
    let action = sync.live_changed(present(deployment(4, "nginx:1.27", "42")), false);
    assert!(matches!(action, SyncAction::Reload(_)), "{action:?}");

    sync.edited(true);
    sync.keep_mine();
    assert_eq!(sync.state(), SyncState::Dirty, "an edit stays an edit");
    assert_eq!(
        sync.live_changed(present(deployment(5, "nginx:1.27", "43")), true),
        SyncAction::Banner,
        "the server's change was never acknowledged"
    );
}

#[test]
fn a_non_conflicting_merge_keeps_both_changes_and_ends_dirty() {
    let (mut sync, base) = loaded();
    // The user changed the image (and added a comment); the server changed the replicas.
    let mine = base
        .text
        .replace("image: nginx:1.27", "image: nginx:1.28 # bumped");
    sync.edited(true);
    sync.live_changed(present(deployment(5, "nginx:1.27", "50")), true);
    let MergeStep::Done(text) = sync.merge(&mine) else {
        panic!("merges cleanly")
    };
    assert!(text.contains("image: nginx:1.28 # bumped"), "{text}");
    assert!(text.contains("replicas: 5"), "{text}");
    assert!(text.contains("resourceVersion: '50'") || text.contains("resourceVersion: \"50\""));
    assert_eq!(sync.state(), SyncState::Dirty);
}

#[test]
fn a_conflicting_merge_surfaces_hunks_picked_per_side() {
    let (mut sync, base) = loaded();
    let mine = base.text.replace("replicas: 3", "replicas: 7");
    sync.edited(true);
    sync.live_changed(present(deployment(5, "nginx:1.27", "50")), true);
    let step = sync.merge(&mine);
    assert!(
        matches!(step, MergeStep::Conflicts(n) if n >= 1),
        "{step:?}"
    );
    assert_eq!(sync.state(), SyncState::Merging);
    assert_eq!(sync.accept(), None, "not every conflict has a side");
    let conflicts = sync.pending_merge().unwrap().conflicts();
    for i in 0..conflicts {
        assert!(sync.choose(i, MergeSide::Mine));
    }
    let text = sync.accept().expect("every conflict has a side");
    assert!(text.contains("replicas: 7"), "{text}");
    assert_eq!(sync.state(), SyncState::Dirty);
}

#[test]
fn editing_during_a_merge_drops_it_back_to_the_banner() {
    let (mut sync, base) = loaded();
    let mine = base.text.replace("replicas: 3", "replicas: 7");
    sync.edited(true);
    sync.live_changed(present(deployment(5, "nginx:1.27", "50")), true);
    sync.merge(&mine);
    sync.edited(true);
    assert_eq!(sync.state(), SyncState::Banner);
    assert!(sync.pending_merge().is_none());
}

#[test]
fn a_status_only_change_keeps_the_banner_and_a_pending_merge_with_its_picks() {
    let (mut sync, base) = loaded();
    let mine = base.text.replace("replicas: 3", "replicas: 7");
    sync.edited(true);
    let raised = deployment(5, "nginx:1.27", "50");
    assert_eq!(
        sync.live_changed(present(raised.clone()), true),
        SyncAction::Banner
    );
    // While the banner is up, a status tick on the change it was raised for is not news.
    let mut tick = raised;
    tick["metadata"]["resourceVersion"] = json!("51");
    tick["status"]["readyReplicas"] = json!(2);
    assert_eq!(
        sync.live_changed(present(tick.clone()), true),
        SyncAction::None
    );
    assert_eq!(sync.state(), SyncState::Banner);

    let step = sync.merge(&mine);
    assert!(
        matches!(step, MergeStep::Conflicts(n) if n >= 1),
        "{step:?}"
    );
    assert!(sync.choose(0, MergeSide::Mine));
    let unresolved = sync.pending_merge().unwrap().unresolved();
    // A status / resourceVersion tick during the merge keeps it and the side already picked.
    tick["metadata"]["resourceVersion"] = json!("52");
    tick["status"]["readyReplicas"] = json!(5);
    let tick = version(tick);
    assert!(
        sync.is_current(&tick),
        "server-owned fields aside, still the merged version"
    );
    assert_eq!(
        sync.live_changed(LiveObject::Present(tick), true),
        SyncAction::None
    );
    assert_eq!(sync.state(), SyncState::Merging);
    let pending = sync.pending_merge().expect("the merge survives the tick");
    assert_eq!(pending.unresolved(), unresolved, "the pick survives too");

    // A real change during the merge still drops it back to the banner.
    let changed = version(deployment(6, "nginx:1.27", "53"));
    assert!(!sync.is_current(&changed));
    assert_eq!(
        sync.live_changed(LiveObject::Present(changed), true),
        SyncAction::Banner
    );
    assert_eq!(sync.state(), SyncState::Banner);
    assert!(sync.pending_merge().is_none());
}

#[test]
fn a_deleted_object_raises_the_banner_and_cannot_be_reloaded() {
    let (mut sync, _) = loaded();
    assert_eq!(
        sync.live_changed(LiveObject::Gone, false),
        SyncAction::Banner
    );
    assert_eq!(sync.reload(), None);
    assert_eq!(sync.merge("x: 1\n"), MergeStep::Unavailable);
    sync.keep_mine();
    assert_eq!(
        sync.live_changed(LiveObject::Gone, true),
        SyncAction::None,
        "a re-list that still finds nothing does not ask again"
    );
}

#[test]
fn an_unloaded_buffer_follows_the_live_object_without_a_banner() {
    let mut sync = LiveSync::new();
    assert_eq!(
        sync.live_changed(present(deployment(3, "nginx:1.27", "1")), true),
        SyncAction::None
    );
    assert!(sync.live().is_some());
    assert_eq!(sync.state(), SyncState::Clean);
}
