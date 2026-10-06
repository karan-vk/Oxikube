//! The CRD side of [`ResourceViews`] (E07-S07): `crd::OpenResources`, and the versions a custom
//! resource table offers its switcher.
//!
//! `crd::OpenResources { cluster, name }` reads the CRD (`ResourceReader::get` on the Tokio
//! bridge), takes the version a table should open ([`CrdInfo::display_version`]: the storage
//! version when it is served, else the newest served one), asks discovery for that kind and opens
//! its table in the cluster's tab. A CRD that serves nothing, or is gone, says so in a toast.
//! `crd::OpenList` is `resource::OpenList` for the CRD kind (see `controller`).
//!
//! The table's version switcher lists what discovery serves for its kind
//! ([`served_versions`]); discovery runs once per connection for the whole window (the cache the
//! sidebar navigation fills), off the UI thread.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{Context, Entity, Task, Window};
use oxikube_domain::OxiError;
use oxikube_domain::access::is_builtin_api_group;
use oxikube_domain::ids::{ClusterId, Gvk};
use oxikube_domain::kinds::ResourceKind;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::controller::{Kinds, ResourceViews, port_id};
use crate::crds::{CrdInfo, crd_gvk, served_versions};
use crate::table::ResourceTable;

/// The tasks of the CRD flows, replaced (so cancelled) by a newer one, never cleared from inside.
#[derive(Default)]
pub(super) struct CrdTasks {
    /// `crd::OpenResources` in flight (a newer one replaces it).
    open: Option<Task<()>>,
    /// The version reads, one per table kind.
    versions: HashMap<(ClusterId, Gvk), Task<()>>,
}

impl ResourceViews {
    /// Opens the table of the custom resources the CRD `name` defines. See the
    /// [module docs](self).
    pub fn open_crd_resources(
        &mut self,
        cluster: &ClusterId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.deps.table.sessions.get(cluster) else {
            return;
        };
        let (Some(reader), Some(discovery)) = (session.resources(), session.discovery()) else {
            self.toast(cluster, Toast::info("The cluster is not connected."), cx);
            return;
        };
        let lookup = name.clone();
        let find = spawn_kube(cx, async move {
            let crd = reader.get(&crd_gvk(), None, &lookup).await?;
            let info = CrdInfo::parse(&crd.json)
                .ok_or_else(|| OxiError::validation(format!("{lookup} is not a CRD")))?;
            let Some(version) = info.display_version() else {
                return Ok(Err(info));
            };
            let gvk = info.gvk(&version.name);
            Ok(match discovery.resolve(&gvk).await? {
                Some(kind) => Ok(kind),
                None => Err(info),
            })
        });
        let cluster = cluster.clone();
        self.crds.open = Some(cx.spawn_in(window, async move |this, cx| {
            let found = match find.await {
                Ok(found) => found,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update_in(cx, |views, window, cx| match found {
                Ok(Ok(kind)) => {
                    views.open_list(&cluster, kind, window, cx);
                }
                Ok(Err(info)) => {
                    let text = format!("{} is not served: there is nothing to list.", info.kind);
                    views.toast(&cluster, Toast::info(text), cx);
                }
                Err(error) => {
                    tracing::warn!(%error, %name, "opening the custom resources of a CRD failed");
                    let text = format!("Could not open {name}: {}", error.message());
                    views.toast(&cluster, Toast::error(text), cx);
                }
            })
            .ok();
        }));
    }

    /// Tells `table` which versions of its kind the cluster serves, when the kind is a custom
    /// one (a built-in kind has one catalogue whatever its version). From the discovery result
    /// this window already holds, else from one discovery run.
    pub(super) fn load_versions(&mut self, table: &Entity<ResourceTable>, cx: &mut Context<Self>) {
        let (cluster, kind) = {
            let table = table.read(cx);
            (table.cluster().clone(), table.kind().clone())
        };
        if is_builtin_api_group(&kind.gvk.group) {
            return;
        }
        let Some(discovery) = self
            .deps
            .table
            .sessions
            .get(&cluster)
            .and_then(|session| session.discovery())
        else {
            return;
        };
        let port = port_id(&discovery);
        let known = self
            .kinds
            .get(&cluster)
            .filter(|known| known.port == port)
            .map(|known| served_versions(&known.kinds, &kind))
            .filter(|versions| !versions.is_empty());
        if let Some(versions) = known {
            table.update(cx, |table, cx| table.set_served_versions(versions, cx));
            return;
        }
        let discover = spawn_kube(cx, async move { discovery.discover().await });
        let (weak, key) = (table.downgrade(), (cluster.clone(), kind.gvk.clone()));
        let task = cx.spawn(async move |this, cx| {
            let kinds = match discover.await {
                Ok(Ok(kinds)) => kinds,
                Ok(Err(error)) => {
                    tracing::warn!(%error, %cluster, "discovery failed: no version switcher");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "discovery task failed");
                    return;
                }
            };
            this.update(cx, |views, cx| {
                let kinds: Arc<[ResourceKind]> = kinds.into();
                let versions = served_versions(&kinds, &kind);
                views.kinds.insert(cluster, Kinds { port, kinds });
                weak.update(cx, |table, cx| table.set_served_versions(versions, cx))
                    .ok();
            })
            .ok();
        });
        self.crds.versions.insert(key, task);
    }

    /// A toast in `cluster`'s tab.
    fn toast(&self, cluster: &ClusterId, toast: Toast, cx: &mut Context<Self>) {
        if let Some(workspace) = self.tab_workspace(cluster, cx) {
            workspace.update(cx, |ws, cx| ws.show_toast(toast, cx));
        }
    }
}
