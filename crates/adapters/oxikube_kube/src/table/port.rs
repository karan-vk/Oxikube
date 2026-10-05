//! `TableFeedPort` for [`KubeResources`].

use async_trait::async_trait;
use oxikube_domain::OxiResult;
use oxikube_domain::ids::Gvk;
use oxikube_ports::{Table, TableFeed, TableFeedPort, TableOptions};

use crate::KubeResources;

#[async_trait]
impl TableFeedPort for KubeResources {
    async fn list_table(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<Table> {
        self.list_table_page(kind, namespace, options).await
    }

    async fn table_feed(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<TableFeed> {
        self.open_table_feed(kind, namespace, options).await
    }
}
