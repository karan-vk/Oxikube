//! Compile-time proof that every data-plane port is object-safe and shareable:
//! each trait is implemented by a stub, stored as `Arc<dyn Trait + Send + Sync>`,
//! borrowed as `&dyn Trait`, and called through the trait object with a `Send`
//! future. `Arc<dyn ResourcePort>` must also upcast to each half.

use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::kinds::ResourceKind;
use oxikube_domain::{ErrorKind, ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::*;
use serde_json::Value;

/// A port implementation where every call fails with `Unsupported`.
struct Stub;

fn stub<T>() -> OxiResult<T> {
    Err(OxiError::unsupported("stub"))
}

#[async_trait]
impl ResourceReader for Stub {
    async fn list(&self, _: &Gvk, _: Option<&str>, _: &ListOptions) -> OxiResult<ListPage> {
        stub()
    }
    async fn list_metadata(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        stub()
    }
    async fn get(&self, _: &Gvk, _: Option<&str>, _: &str) -> OxiResult<Resource> {
        stub()
    }
    async fn get_opt(&self, _: &Gvk, _: Option<&str>, _: &str) -> OxiResult<Option<Resource>> {
        stub()
    }
    async fn watch(&self, _: &Gvk, _: Option<&str>, _: &WatchOptions) -> OxiResult<WatchFeed> {
        stub()
    }
    async fn get_scale(&self, _: &Gvk, _: Option<&str>, _: &str) -> OxiResult<Scale> {
        stub()
    }
    async fn get_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
    ) -> OxiResult<Value> {
        stub()
    }
}

#[async_trait]
impl ResourceWriter for Stub {
    async fn create(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        stub()
    }
    async fn replace(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        stub()
    }
    async fn patch(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Patch,
        _: &WriteOptions,
    ) -> OxiResult<Resource> {
        stub()
    }
    async fn delete(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &DeleteOptions,
    ) -> OxiResult<DeleteOutcome> {
        stub()
    }
    async fn delete_collection(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &ListOptions,
        _: &DeleteOptions,
    ) -> OxiResult<DeleteCollectionOutcome> {
        stub()
    }
    async fn scale(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: i32,
        _: &WriteOptions,
    ) -> OxiResult<Scale> {
        stub()
    }
    async fn evict(&self, _: &str, _: &str, _: &DeleteOptions) -> OxiResult<()> {
        stub()
    }
    async fn create_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        stub()
    }
    async fn patch_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Patch,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        stub()
    }
    async fn replace_subresource(
        &self,
        _: &Gvk,
        _: Option<&str>,
        _: &str,
        _: &Subresource,
        _: &Value,
        _: &WriteOptions,
    ) -> OxiResult<Value> {
        stub()
    }
}

#[async_trait]
impl DiscoveryPort for Stub {
    async fn discover(&self) -> OxiResult<Vec<ResourceKind>> {
        stub()
    }
    async fn resolve(&self, _: &Gvk) -> OxiResult<Option<ResourceKind>> {
        stub()
    }
    async fn server_version(&self) -> OxiResult<ServerVersion> {
        stub()
    }
}

#[async_trait]
impl TableFeedPort for Stub {
    async fn list_table(&self, _: &Gvk, _: Option<&str>, _: &TableOptions) -> OxiResult<Table> {
        stub()
    }
    async fn table_feed(&self, _: &Gvk, _: Option<&str>, _: &TableOptions) -> OxiResult<TableFeed> {
        stub()
    }
}

#[async_trait]
impl LogPort for Stub {
    async fn stream_logs(&self, _: &str, _: &str, _: &LogOptions) -> OxiResult<LogStream> {
        stub()
    }
}

#[async_trait]
impl ExecPort for Stub {
    async fn exec(&self, _: &ExecTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        stub()
    }
    async fn attach(&self, _: &AttachTarget) -> OxiResult<Box<dyn TerminalBackend>> {
        stub()
    }
    async fn create_debug_container(
        &self,
        _: &DebugContainerSpec,
    ) -> OxiResult<Box<dyn TerminalBackend>> {
        stub()
    }
    async fn node_shell(&self, _: &NodeShellSpec) -> OxiResult<Box<dyn TerminalBackend>> {
        stub()
    }
    async fn sweep_node_shells(&self, _: &str, _: std::time::Duration) -> OxiResult<Vec<String>> {
        stub()
    }
    async fn release_node_shells(&self) -> OxiResult<usize> {
        stub()
    }
}

#[async_trait]
impl ExecStreamPort for Stub {
    async fn exec_session(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &ExecOptions,
    ) -> OxiResult<ExecSession> {
        stub()
    }
    async fn attach_session(&self, _: &str, _: &str, _: &ExecOptions) -> OxiResult<ExecSession> {
        stub()
    }
}

#[async_trait]
impl TerminalBackend for Stub {
    async fn write(&self, _: &[u8]) -> OxiResult<()> {
        stub()
    }
    async fn resize(&self, _: TerminalSize) -> OxiResult<()> {
        stub()
    }
    fn output_stream(&self) -> futures::stream::BoxStream<'static, BackendEvent> {
        futures::StreamExt::boxed(futures::stream::empty())
    }
    async fn kill(&self) -> OxiResult<()> {
        stub()
    }
}

#[async_trait]
impl PortForwardPort for Stub {
    async fn forward(&self, _: &str, _: &str, _: u16) -> OxiResult<PortForwardConnection> {
        stub()
    }
}

fn assert_send_sync<T: ?Sized + Send + Sync>() {}

fn assert_send<T: Send>(value: T) -> T {
    value
}

/// `&dyn Trait` compiles for every port.
fn _borrow_each(
    _: &dyn ResourceReader,
    _: &dyn ResourceWriter,
    _: &dyn ResourcePort,
    _: &dyn DiscoveryPort,
    _: &dyn TableFeedPort,
    _: &dyn LogPort,
    _: &dyn ExecPort,
    _: &dyn ExecStreamPort,
    _: &dyn TerminalBackend,
    _: &dyn PortForwardPort,
) {
}

fn pod() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

fn assert_unsupported<T>(result: OxiResult<T>) {
    match result {
        Err(e) => assert_eq!(e.kind(), ErrorKind::Unsupported),
        Ok(_) => panic!("stub returned Ok"),
    }
}

#[test]
fn every_port_is_dyn_send_sync() {
    assert_send_sync::<dyn ResourceReader>();
    assert_send_sync::<dyn ResourceWriter>();
    assert_send_sync::<dyn ResourcePort>();
    assert_send_sync::<dyn DiscoveryPort>();
    assert_send_sync::<dyn TableFeedPort>();
    assert_send_sync::<dyn LogPort>();
    assert_send_sync::<dyn ExecPort>();
    assert_send_sync::<dyn ExecStreamPort>();
    assert_send_sync::<dyn TerminalBackend>();
    assert_send_sync::<Box<dyn TerminalBackend>>();
    assert_send_sync::<dyn PortForwardPort>();
}

#[test]
fn resource_port_is_callable_through_arc_dyn() {
    let port: Arc<dyn ResourcePort + Send + Sync> = Arc::new(Stub);
    let kind = pod();
    block_on(async {
        assert_unsupported(
            assert_send(port.list(&kind, Some("default"), &ListOptions::default())).await,
        );
        assert_unsupported(
            port.list_metadata(&kind, None, &ListOptions::default())
                .await,
        );
        assert_unsupported(port.get(&kind, Some("default"), "web-0").await);
        assert_unsupported(port.get_opt(&kind, Some("default"), "web-0").await);
        assert_unsupported(port.watch(&kind, None, &WatchOptions::default()).await);
        assert_unsupported(port.get_scale(&kind, Some("d"), "n").await);
        assert_unsupported(
            port.get_subresource(&kind, Some("d"), "n", &Subresource::Status)
                .await,
        );
        let body = Value::Null;
        let write = WriteOptions::dry_run();
        assert_unsupported(assert_send(port.create(&kind, Some("d"), &body, &write)).await);
        assert_unsupported(port.replace(&kind, Some("d"), "n", &body, &write).await);
        assert_unsupported(
            port.patch(&kind, Some("d"), "n", &Patch::merge(body.clone()), &write)
                .await,
        );
        let del = DeleteOptions::dry_run();
        assert_unsupported(port.delete(&kind, Some("d"), "n", &del).await);
        assert_unsupported(
            port.delete_collection(&kind, Some("d"), &ListOptions::default(), &del)
                .await,
        );
        assert_unsupported(port.scale(&kind, Some("d"), "n", 3, &write).await);
        assert_unsupported(port.evict("d", "n", &del).await);
        let sub = Subresource::EphemeralContainers;
        assert_unsupported(
            port.create_subresource(&kind, Some("d"), "n", &sub, &body, &write)
                .await,
        );
        assert_unsupported(
            port.patch_subresource(
                &kind,
                Some("d"),
                "n",
                &sub,
                &Patch::merge(body.clone()),
                &write,
            )
            .await,
        );
        assert_unsupported(
            port.replace_subresource(&kind, Some("d"), "n", &sub, &body, &write)
                .await,
        );
    });
}

#[test]
fn resource_port_upcasts_to_reader_and_writer() {
    let port: Arc<dyn ResourcePort> = Arc::new(Stub);
    let reader: Arc<dyn ResourceReader> = port.clone();
    let writer: Arc<dyn ResourceWriter> = port;
    block_on(async {
        assert_unsupported(reader.get(&pod(), None, "x").await);
        assert_unsupported(writer.evict("d", "n", &DeleteOptions::default()).await);
    });
}

#[test]
fn other_ports_are_callable_through_arc_dyn() {
    let discovery: Arc<dyn DiscoveryPort + Send + Sync> = Arc::new(Stub);
    let table: Arc<dyn TableFeedPort + Send + Sync> = Arc::new(Stub);
    let logs: Arc<dyn LogPort + Send + Sync> = Arc::new(Stub);
    let exec: Arc<dyn ExecPort + Send + Sync> = Arc::new(Stub);
    let forward: Arc<dyn PortForwardPort + Send + Sync> = Arc::new(Stub);
    let kind = pod();
    block_on(async {
        assert_unsupported(assert_send(discovery.discover()).await);
        assert_unsupported(discovery.resolve(&kind).await);
        assert_unsupported(discovery.server_version().await);
        assert_unsupported(
            assert_send(table.list_table(&kind, None, &TableOptions::default())).await,
        );
        assert_unsupported(
            table
                .table_feed(&kind, None, &TableOptions::default())
                .await,
        );
        assert_unsupported(
            assert_send(logs.stream_logs("d", "web-0", &LogOptions::follow())).await,
        );
        let pod = ResourceRef::new(
            ClusterId::new("kubeconfig", &ContextName::new("kind")),
            kind.clone(),
            Some("d".into()),
            "web-0",
        );
        let target = ExecTarget::interactive(pod.clone(), vec!["sh".to_owned()]);
        assert_unsupported(assert_send(exec.exec(&target)).await);
        assert_unsupported(exec.attach(&AttachTarget::interactive(pod.clone())).await);
        assert_unsupported(
            assert_send(exec.create_debug_container(&DebugContainerSpec::new(pod, "busybox")))
                .await,
        );
        assert_unsupported(assert_send(exec.node_shell(&NodeShellSpec::new("node-1"))).await);
        assert_unsupported(
            assert_send(exec.sweep_node_shells("kube-system", std::time::Duration::ZERO)).await,
        );
        assert_unsupported(assert_send(exec.release_node_shells()).await);
        let streams: Arc<dyn ExecStreamPort + Send + Sync> = Arc::new(Stub);
        assert_unsupported(
            assert_send(streams.exec_session(
                "d",
                "web-0",
                &["sh".to_owned()],
                &ExecOptions::interactive(),
            ))
            .await,
        );
        assert_unsupported(
            streams
                .attach_session("d", "web-0", &ExecOptions::default())
                .await,
        );
        let backend: Box<dyn TerminalBackend> = Box::new(Stub);
        assert_unsupported(assert_send(backend.write(b"x")).await);
        assert_unsupported(backend.resize(TerminalSize::new(80, 24)).await);
        assert_unsupported(backend.kill().await);
        assert_unsupported(assert_send(forward.forward("d", "web-0", 8080)).await);
    });
}
