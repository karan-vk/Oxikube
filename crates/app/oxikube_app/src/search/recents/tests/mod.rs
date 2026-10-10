//! Tests of the persisted recents and jump history over `FakeStatePort`.

mod commands;
mod failure;
mod jump;
mod writer;

use std::sync::Arc;

use oxikube_domain::command::CommandId;
use oxikube_domain::ids::{ClusterId, ContextName};
use oxikube_ports::{StateKey, StatePort};
use oxikube_testkit::fakes::{FakeStatePort, StateCall};
use serde_json::Value;

use super::StateRecents;

pub(super) fn block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
    futures::executor::block_on(fut)
}

pub(super) fn fake() -> Arc<FakeStatePort> {
    Arc::new(FakeStatePort::new())
}

pub(super) fn recents(state: &Arc<FakeStatePort>) -> StateRecents {
    StateRecents::new(state.clone())
}

pub(super) fn cluster(name: &str) -> ClusterId {
    ClusterId::new("/kubeconfig", &ContextName::new(name))
}

pub(super) fn stored(state: &FakeStatePort, key: &str) -> Option<Value> {
    block_on(state.kv_get(&StateKey::new(key).unwrap())).unwrap()
}

/// The `kv_set` calls made so far.
pub(super) fn writes(state: &FakeStatePort) -> Vec<(String, Value)> {
    state
        .recorded_calls()
        .into_iter()
        .filter_map(|call| match call {
            StateCall::KvSet(key, value) => Some((key.as_str().to_string(), value)),
            _ => None,
        })
        .collect()
}

/// Real ids, in the order the tests use them.
pub(super) const DELETE: CommandId = CommandId::POD_DELETE;
pub(super) const ZOOM: CommandId = CommandId::VIEW_ZOOM_IN;
pub(super) const SHELL: CommandId = CommandId::POD_ATTACH;
