//! Resolving a Table collection and fetching one page of it.

use std::time::{Duration, Instant};

use kube::Client;
use kube::api::ListParams;
use kube::core::{DynamicObject, Resource as _};
use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::{IncludeObject, Table, TableOptions};
use tracing::debug;

use super::convert::{self, Page};
use super::{request, wire};
use crate::KubeResources;
use crate::resources::{bad_object, deadline, list_error, list_params, namespace_of};

/// A resolved Table collection: where requests go and what rows embed.
#[derive(Debug, Clone)]
pub(crate) struct Target {
    /// The kind, for logs.
    pub(crate) gvk: Gvk,
    /// Collection URL path, e.g. `/apis/test.oxikube.dev/v1/namespaces/x/widgets`.
    pub(crate) path: String,
    /// `includeObject` of every request.
    pub(crate) include: IncludeObject,
    /// Whether discovery lists the `watch` verb for the kind.
    pub(crate) watchable: bool,
}

impl KubeResources {
    /// Resolves `kind` through discovery: it must be served with `list`, and `namespace` must
    /// fit its scope (`None` or `""` lists across namespaces).
    pub(crate) async fn table_target(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        include: IncludeObject,
    ) -> OxiResult<Target> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::List, false).await?;
        Ok(Target {
            gvk: kind.clone(),
            path: DynamicObject::url_path(&resource, namespace),
            include,
            watchable: self.serves(kind, Verb::Watch),
        })
    }

    /// One page as a Table; see [`TableFeedPort::list_table`](oxikube_ports::TableFeedPort).
    ///
    /// Selectors, `limit`, `continue` and `resourceVersion` come from `options.list` (with
    /// the same rules as `ResourceReader::list`); the deadline is `options.list.timeout_secs`
    /// or [`TableConfig::request_timeout`](super::TableConfig::request_timeout).
    pub(crate) async fn list_table_page(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &TableOptions,
    ) -> OxiResult<Table> {
        let target = self
            .table_target(kind, namespace, options.include_object)
            .await?;
        let params = list_params(&options.list)?;
        let timeout = deadline(&options.list).unwrap_or(self.config().table.request_timeout);
        let page = fetch_page(self.client(), &target, &params, timeout).await?;
        Ok(Table {
            columns: page.columns,
            rows: page.rows,
            continue_token: page.continue_token,
            resource_version: page.resource_version,
            source: page.source,
        })
    }
}

/// Fetches and decodes one page within `timeout`. An expired continue token is an error for
/// which [`is_list_expired`](crate::is_list_expired) holds.
pub(crate) async fn fetch_page(
    client: &Client,
    target: &Target,
    params: &ListParams,
    timeout: Duration,
) -> OxiResult<Page> {
    let started = Instant::now();
    let request = request::list(&target.path, params, target.include)?;
    let text = match tokio::time::timeout(timeout, client.request_text(request)).await {
        Ok(response) => response.map_err(|e| list_error(&e))?,
        Err(_) => {
            return Err(OxiError::timeout(format!(
                "table list did not complete within {}s",
                timeout.as_secs()
            )));
        }
    };
    // Our own decode error: kube's `Client::request` would log the whole body on failure.
    let table: wire::Table = serde_json::from_str(&text).map_err(|e| bad_object("table", e))?;
    let bytes = text.len();
    drop(text);
    let page = convert::page(table, target.include)?;
    debug!(
        kind = %target.gvk,
        rows = page.rows.len(),
        bytes,
        server_table = page.source.is_server(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "table: page"
    );
    Ok(page)
}
