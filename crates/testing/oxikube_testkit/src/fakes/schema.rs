//! Fake [`SchemaPort`](oxikube_ports::SchemaPort) with scripted schemas.

use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::schema::JsonSchema;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::SchemaPort;

use crate::script::{CallLog, Script};

/// Queued responses for each [`FakeSchemaPort`] method.
#[derive(Debug, Default)]
pub struct SchemaScripts {
    /// `schema_for`.
    pub schema_for: Script<Arc<JsonSchema>>,
    /// `invalidate`.
    pub invalidate: Script<()>,
}

/// One call made on a [`FakeSchemaPort`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaCall {
    /// `schema_for(cluster, gvk)`.
    SchemaFor(ClusterId, Gvk),
    /// `invalidate(cluster)`.
    Invalidate(ClusterId),
}

/// Fake `SchemaPort` with scripted schemas for the validator (E10-S03) onwards.
///
/// Fallbacks: `schema_for` serves the stored schemas under (cluster, GVK) and
/// fails `NotFound` otherwise; `invalidate` only records the call (the stored
/// schemas stand for the server, which still has them). A scripted response
/// replaces the fallback for that call entirely.
#[derive(Debug, Default)]
pub struct FakeSchemaPort {
    script: SchemaScripts,
    calls: CallLog<SchemaCall>,
    schemas: parking_lot::Mutex<std::collections::HashMap<(ClusterId, Gvk), Arc<JsonSchema>>>,
}

fake_plumbing!(FakeSchemaPort, SchemaScripts, SchemaCall);

impl FakeSchemaPort {
    /// A fake with nothing stored or scripted.
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores `schema` for (`cluster`, `gvk`), served when nothing is scripted.
    pub fn insert(&self, cluster: ClusterId, gvk: Gvk, schema: Arc<JsonSchema>) {
        self.schemas.lock().insert((cluster, gvk), schema);
    }
}

#[async_trait]
impl SchemaPort for FakeSchemaPort {
    async fn schema_for(&self, cluster: &ClusterId, gvk: &Gvk) -> OxiResult<Arc<JsonSchema>> {
        self.calls
            .record(SchemaCall::SchemaFor(cluster.clone(), gvk.clone()));
        self.script.schema_for.next_or_else(|| {
            self.schemas
                .lock()
                .get(&(cluster.clone(), gvk.clone()))
                .cloned()
                .ok_or_else(|| OxiError::not_found(format!("no scripted schema for {gvk}")))
        })
    }

    async fn invalidate(&self, cluster: &ClusterId) -> OxiResult<()> {
        self.calls.record(SchemaCall::Invalidate(cluster.clone()));
        self.script.invalidate.next_or_else(|| Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use oxikube_domain::ErrorKind;
    use oxikube_domain::ids::ContextName;

    use super::*;

    fn cluster() -> ClusterId {
        ClusterId::new("/kubeconfig", &ContextName::new("kind-oxikube"))
    }

    #[test]
    fn stored_schemas_serve_and_survive_invalidate() {
        let fake = FakeSchemaPort::new();
        let gvk = Gvk::new("apps", "v1", "Deployment");
        let schema = Arc::new(JsonSchema::any());
        fake.insert(cluster(), gvk.clone(), schema.clone());
        assert!(Arc::ptr_eq(
            &block_on(fake.schema_for(&cluster(), &gvk)).unwrap(),
            &schema
        ));
        block_on(fake.invalidate(&cluster())).unwrap();
        assert!(Arc::ptr_eq(
            &block_on(fake.schema_for(&cluster(), &gvk)).unwrap(),
            &schema
        ));
        assert_eq!(
            block_on(fake.schema_for(&cluster(), &Gvk::new("apps", "v1", "StatefulSet")))
                .unwrap_err()
                .kind(),
            ErrorKind::NotFound
        );
        assert_eq!(
            fake.recorded_calls(),
            vec![
                SchemaCall::SchemaFor(cluster(), gvk.clone()),
                SchemaCall::Invalidate(cluster()),
                SchemaCall::SchemaFor(cluster(), gvk),
                SchemaCall::SchemaFor(cluster(), Gvk::new("apps", "v1", "StatefulSet")),
            ]
        );
    }

    #[test]
    fn scripted_responses_replace_the_store() {
        let fake = FakeSchemaPort::new();
        let gvk = Gvk::new("", "v1", "Pod");
        fake.script()
            .schema_for
            .push_err(OxiError::forbidden("nope"));
        assert_eq!(
            block_on(fake.schema_for(&cluster(), &gvk))
                .unwrap_err()
                .kind(),
            ErrorKind::Forbidden
        );
    }
}
