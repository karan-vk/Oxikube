//! Fixtures shared by the diff-vs-live tests and the live bench.

use std::sync::Arc;

use gpui::App;
use oxikube_domain::ids::ClusterId;
use oxikube_editor::view::LiveSource;
use oxikube_ports::ResourceReader;
use oxikube_testkit::fakes::FakeResourcePort;

/// A [`LiveSource`] serving one cluster's objects from a fake resource port.
pub struct FakeLive {
    cluster: ClusterId,
    port: Arc<FakeResourcePort>,
}

impl FakeLive {
    /// The objects of `port`, as `cluster`'s.
    pub fn new(cluster: ClusterId, port: Arc<FakeResourcePort>) -> Self {
        Self { cluster, port }
    }
}

impl LiveSource for FakeLive {
    fn cluster(&self) -> &ClusterId {
        &self.cluster
    }

    fn reader(&self, _: &App) -> Option<Arc<dyn ResourceReader>> {
        Some(self.port.clone())
    }
}
