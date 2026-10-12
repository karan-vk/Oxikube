//! [`fetch_count`]: one cheap request for a kind's count, the fallback behind a tile whose kind
//! has no feed (the watch budget refused it, or nothing opened it).
//!
//! The request is a metadata list with `limit=1`: the API server sends one object and, when
//! there are more, `remainingItemCount`, so the cost does not grow with the kind and no object
//! body crosses the wire. A server that omits the remaining count (it does for some selectors
//! and aggregated APIs) leaves the answer a lower bound, which [`FetchedCount::exact`] says.

use oxikube_domain::OxiResult;
use oxikube_domain::session::{NamespaceSelection, WatchScope};
use oxikube_ports::{ListOptions, ResourceReader};

use super::CountTarget;

/// A count read from the API server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FetchedCount {
    /// How many objects there are, or at least that many when not [`exact`](Self::exact).
    pub total: u64,
    /// Whether the server reported everything it has (a lower bound otherwise).
    pub exact: bool,
}

/// The count of `target` under `selection`, one request per namespace of the selection (one for
/// a cluster-wide scope).
///
/// # Errors
///
/// The first failing request's error: `Forbidden` for a user who may not list the kind, which a
/// caller shows as "no access" and never as zero.
pub async fn fetch_count(
    reader: &dyn ResourceReader,
    target: &CountTarget,
    selection: &NamespaceSelection,
) -> OxiResult<FetchedCount> {
    let scope = WatchScope::derive(selection, target.scope);
    let namespaces: Vec<Option<&str>> = match &scope {
        WatchScope::Cluster => vec![None],
        WatchScope::Namespaces(names) => names.iter().map(|n| Some(n.as_str())).collect(),
    };
    let mut sum = FetchedCount {
        total: 0,
        exact: true,
    };
    for namespace in namespaces {
        let page = reader
            .list_metadata(&target.gvk, namespace, &ListOptions::default().limit(1))
            .await?;
        let held = page.items.len() as u64;
        match (page.has_more(), page.remaining_item_count) {
            (false, _) => sum.total += held,
            (true, Some(remaining)) => sum.total += held + u64::try_from(remaining).unwrap_or(0),
            // More than the page, and the server did not say how many: at least one more.
            (true, None) => {
                sum.total += held + 1;
                sum.exact = false;
            }
        }
    }
    Ok(sum)
}
