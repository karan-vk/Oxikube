//! List calls: one page, metadata-only, and the page-until-exhausted helper.

use std::time::Instant;

use oxikube_domain::ids::Gvk;
use oxikube_domain::kinds::Verb;
use oxikube_domain::{ObjectMeta, OxiError, OxiResult, Resource};
use oxikube_ports::{ListOptions, ListPage};
use tracing::debug;

use super::backend::RawPage;
use super::error::{bad_object, is_list_expired, list_error};
use super::params::list_params;
use super::{KubeResources, namespace_of};

/// Capacity hint cap: a server-reported remaining count is advisory.
const MAX_RESERVE: usize = 100_000;

impl KubeResources {
    /// One page of objects; see [`ResourceReader::list`](oxikube_ports::ResourceReader::list).
    ///
    /// `options.limit` of `None` asks the server for everything in one response (use
    /// [`list_all`](Self::list_all) for paging). A stale continue token or compacted `Exact`
    /// version is an error for which [`is_list_expired`] holds.
    pub(super) async fn list_page(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::List, false).await?;
        let params = list_params(options)?;
        let api = self.kind_api(&resource, namespace);
        let strip = self.config.list_managed_fields.strips();
        let raw = api.list(&params, strip).await.map_err(|e| list_error(&e))?;
        into_resources(raw)
    }

    /// One page of object metadata (`PartialObjectMetadata`); see
    /// [`ResourceReader::list_metadata`](oxikube_ports::ResourceReader::list_metadata).
    pub(super) async fn list_meta_page(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<ObjectMeta>> {
        let namespace = namespace_of(namespace);
        let resource = self.target(kind, namespace, Verb::List, false).await?;
        let params = list_params(options)?;
        let raw = self
            .dynamic(&resource, namespace)
            .list_metadata(&params)
            .await
            .map_err(|e| list_error(&e))?;
        let page = into_resources(raw)?;
        Ok(ListPage {
            items: page.items.into_iter().map(|r| r.meta).collect(),
            continue_token: page.continue_token,
            resource_version: page.resource_version,
            remaining_item_count: page.remaining_item_count,
        })
    }

    /// Every object of `kind`, fetched page by page until the server stops returning a
    /// continue token.
    ///
    /// The page size is `options.limit` or [`ResourcesConfig::page_size`](super::ResourcesConfig::page_size). Each page is
    /// converted as it arrives. If the server expires the continue token mid-list (HTTP 410
    /// after a long pause or heavy churn), the list restarts from the first page, up to
    /// [`ResourcesConfig::max_restarts`](super::ResourcesConfig::max_restarts) times, discarding what was collected so the result
    /// is one consistent snapshot; then the error is returned.
    ///
    /// The returned page has no continue token, and its `resource_version` is the
    /// collection's, so a watch can start from it. `options.continue_token` must be unset.
    ///
    /// # Errors
    ///
    /// As `ResourceReader::list`, plus `Validation` if a continue token was given.
    pub async fn list_all(
        &self,
        kind: &Gvk,
        namespace: Option<&str>,
        options: &ListOptions,
    ) -> OxiResult<ListPage<Resource>> {
        if options.continue_token.is_some() {
            return Err(OxiError::validation(
                "list_all starts from the first page; use list to continue a token",
            ));
        }
        let started = Instant::now();
        let base = ListOptions {
            limit: options.limit.or(Some(self.config.page_size)),
            ..options.clone()
        };
        let mut restarts = 0;
        'restart: loop {
            let mut items: Vec<Resource> = Vec::new();
            let mut resource_version = None;
            let mut next = None;
            let mut pages = 0u32;
            loop {
                let mut request = base.clone();
                request.continue_token = next.take();
                let page = match self.list_page(kind, namespace, &request).await {
                    Ok(page) => page,
                    // A continuation whose token expired: the snapshot is gone, start over.
                    Err(err)
                        if pages > 0
                            && is_list_expired(&err)
                            && restarts < self.config.max_restarts =>
                    {
                        restarts += 1;
                        debug!(kind = %kind, restarts, "list: continue token expired, restarting");
                        continue 'restart;
                    }
                    Err(err) => return Err(err),
                };
                pages += 1;
                if pages == 1 {
                    let hint = page.remaining_item_count.unwrap_or(0).max(0) as usize;
                    items.reserve(page.items.len() + hint.min(MAX_RESERVE));
                }
                let more = page.has_more();
                items.extend(page.items);
                resource_version = page.resource_version.or(resource_version);
                if !more {
                    debug!(
                        kind = %kind,
                        pages,
                        items = items.len(),
                        restarts,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "list: complete"
                    );
                    return Ok(ListPage {
                        items,
                        continue_token: None,
                        resource_version,
                        remaining_item_count: None,
                    });
                }
                next = page.continue_token;
            }
        }
    }
}

/// Server JSON to domain objects, one page.
fn into_resources(raw: RawPage) -> OxiResult<ListPage<Resource>> {
    let RawPage {
        items,
        continue_token,
        resource_version,
        remaining_item_count,
    } = raw;
    let items = items
        .into_iter()
        .map(|json| Resource::from_json(json).map_err(|e| bad_object("object", e)))
        .collect::<OxiResult<Vec<_>>>()?;
    Ok(ListPage {
        items,
        continue_token,
        resource_version,
        remaining_item_count,
    })
}
