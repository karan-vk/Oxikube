//! [`RelistDiff`]: turns a relist into the deltas that changed, against the reflector store.
//!
//! During a relist the store still holds the previous state (kube's reflector writer
//! buffers the new list and swaps it in on `InitDone`), so each relisted object is compared
//! with its predecessor as it arrives, and what was not seen is deleted just before the swap.

use std::collections::HashSet;

use kube::runtime::reflector::{Lookup, ObjectRef, Store};
use oxikube_domain::Resource;
use oxikube_ports::Delta;

use super::object::FeedObject;

/// The diff of one relist in progress.
#[derive(Default)]
pub(super) struct RelistDiff {
    seen: HashSet<ObjectRef<FeedObject>>,
}

impl RelistDiff {
    /// The deltas for one relisted object, pushed onto `out`: nothing when the store already
    /// has this exact version, `Applied` when it is new or changed, and a `Deleted` of the old
    /// object first when the name now belongs to a different UID (deleted and re-created
    /// while the watch was down).
    pub(super) fn object(
        &mut self,
        store: &Store<FeedObject>,
        object: FeedObject,
        out: &mut Vec<Delta<Resource>>,
    ) {
        let key = object.to_object_ref(());
        let previous = store.get(&key);
        self.seen.insert(key);
        match previous {
            Some(old) if old.meta.uid != object.meta.uid => {
                out.push(Delta::Deleted(old.0.clone()));
                out.push(Delta::Applied(object.0));
            }
            Some(old) if old.meta.resource_version == object.meta.resource_version => {}
            _ => out.push(Delta::Applied(object.0)),
        }
    }

    /// The `Deleted` deltas for every stored object the relist did not return. Call before
    /// the writer applies `InitDone`.
    pub(super) fn finish(self, store: &Store<FeedObject>, out: &mut Vec<Delta<Resource>>) {
        for old in store.state() {
            if !self.seen.contains(&old.to_object_ref(())) {
                out.push(Delta::Deleted(old.0.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::runtime::reflector::store::Writer;
    use kube::runtime::watcher::Event;
    use serde_json::json;

    fn pod(name: &str, uid: &str, rv: &str) -> FeedObject {
        FeedObject(
            Resource::from_json(json!({
                "apiVersion": "v1", "kind": "Pod",
                "metadata": {"name": name, "namespace": "ns", "uid": uid, "resourceVersion": rv},
            }))
            .unwrap(),
        )
    }

    fn label(delta: &Delta<Resource>) -> String {
        match delta {
            Delta::Applied(r) => format!(
                "+{}@{}",
                r.name(),
                r.meta.resource_version.as_deref().unwrap()
            ),
            Delta::Deleted(r) => format!(
                "-{}@{}",
                r.name(),
                r.meta.resource_version.as_deref().unwrap()
            ),
            Delta::Restarted(_) => "restart".into(),
        }
    }

    #[test]
    fn a_relist_yields_only_what_changed() {
        let mut writer: Writer<FeedObject> = Writer::default();
        let store = writer.as_reader();
        writer.apply_watcher_event(&Event::Init);
        for p in [
            pod("same", "u1", "1"),
            pod("changed", "u2", "2"),
            pod("gone", "u3", "3"),
            pod("recreated", "u4", "4"),
        ] {
            writer.apply_watcher_event(&Event::InitApply(p));
        }
        writer.apply_watcher_event(&Event::InitDone);

        let mut diff = RelistDiff::default();
        let mut out = Vec::new();
        writer.apply_watcher_event(&Event::Init);
        for p in [
            pod("same", "u1", "1"),
            pod("changed", "u2", "7"),
            pod("new", "u5", "8"),
            pod("recreated", "u9", "9"),
        ] {
            writer.apply_watcher_event(&Event::InitApply(p.clone()));
            diff.object(&store, p, &mut out);
        }
        diff.finish(&store, &mut out);
        writer.apply_watcher_event(&Event::InitDone);

        let got: Vec<_> = out.iter().map(label).collect();
        assert_eq!(
            got,
            vec![
                "+changed@7",
                "+new@8",
                "-recreated@4",
                "+recreated@9",
                "-gone@3"
            ]
        );
        assert_eq!(store.len(), 4);
    }
}
