//! Integrations: [`IntegrationPort`], the plug-in point for optional packs.
//!
//! An *integration* (Argo CD first, Flux later) is detected per cluster and,
//! when present, contributes a sidebar section, commands, agent tools,
//! context providers and settings. Nothing about Argo CD or Flux lives in the
//! core: `oxikube_argocd` implements this trait and `oxikube_app::integrations`
//! holds the registry (ADR 0009).
//!
//! # Declarative, not views
//!
//! The port returns *data*. [`SidebarModel`] names sections, items and icons
//! (by name); the UI crate for the integration renders them. No GPUI type
//! crosses this boundary (ADR 0007: extensions never contribute UI).
//!
//! # Detection
//!
//! [`IntegrationPort::detect`] receives an [`IntegrationSession`], a plain-data
//! snapshot of the cluster session (id, state, the kinds the API server
//! serves) and answers with the capabilities the integration offers there
//! (for example [`Capabilities::ARGO`] when the `Application` CRD is served).
//! Empty means "not present": the app hides the integration. An adapter may
//! probe further (reach an Argo CD server) because `detect` is async.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use oxikube_domain::command::{CommandId, CommandMeta};
use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
use oxikube_domain::session::ClusterSessionState;
use oxikube_domain::{Capabilities, OxiResult};
use serde::Serialize;
use serde_json::Value;

use crate::context::ContextProviderPort;
use crate::tool::ToolPort;

/// What [`IntegrationPort::detect`] looks at: the cluster session, as data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationSession {
    /// The cluster.
    pub cluster: ClusterId,
    /// The kubeconfig context in use.
    pub context: ContextName,
    /// The session's current state; detection normally requires a connected one.
    pub state: ClusterSessionState,
    /// The kinds the API server serves (from discovery), used to spot CRDs
    /// without another round trip.
    pub served_kinds: BTreeSet<Gvk>,
}

impl IntegrationSession {
    /// A session snapshot.
    pub fn new(
        cluster: ClusterId,
        context: ContextName,
        state: ClusterSessionState,
        served_kinds: impl IntoIterator<Item = Gvk>,
    ) -> Self {
        Self {
            cluster,
            context,
            state,
            served_kinds: served_kinds.into_iter().collect(),
        }
    }

    /// Whether the server serves `kind`.
    pub fn serves(&self, kind: &Gvk) -> bool {
        self.served_kinds.contains(kind)
    }

    /// Whether the session is connected (ready or degraded).
    pub fn is_connected(&self) -> bool {
        self.state.phase().is_connected()
    }
}

/// The sidebar an integration contributes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct SidebarModel {
    /// Sections, top to bottom.
    pub sections: Vec<SidebarSection>,
}

/// A titled group of sidebar items.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SidebarSection {
    /// Stable id, unique within the integration.
    pub id: String,
    /// Heading.
    pub title: String,
    /// Icon name resolved by the UI's icon set; `None` for no icon.
    pub icon: Option<String>,
    /// Entries, top to bottom.
    pub items: Vec<SidebarItem>,
}

/// One sidebar entry. Activating it runs a command (every user action is a
/// `Command`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SidebarItem {
    /// Stable id, unique within the section.
    pub id: String,
    /// Label.
    pub title: String,
    /// Icon name resolved by the UI's icon set; `None` for no icon.
    pub icon: Option<String>,
    /// The command to dispatch.
    pub command: CommandId,
    /// Capabilities that must be present for the item to show.
    pub needs: Capabilities,
}

impl SidebarModel {
    /// A sidebar with one section.
    pub fn single(section: SidebarSection) -> Self {
        Self {
            sections: vec![section],
        }
    }

    /// The sidebar filtered to `have`: items whose `needs` are not met are
    /// dropped, and so are sections left empty.
    pub fn visible_with(&self, have: Capabilities) -> SidebarModel {
        let sections = self
            .sections
            .iter()
            .filter_map(|s| {
                let items: Vec<_> = s
                    .items
                    .iter()
                    .filter(|i| i.needs.satisfied_by(have))
                    .cloned()
                    .collect();
                (!items.is_empty()).then(|| SidebarSection { items, ..s.clone() })
            })
            .collect();
        SidebarModel { sections }
    }
}

/// An optional capability pack. Implemented per integration in its adapter
/// crate and registered with `oxikube_app::integrations`.
///
/// Everything except [`detect`](Self::detect) is static data: it does not
/// depend on the cluster, so the app can register commands and tools once and
/// let the capabilities from `detect` decide what is visible.
#[async_trait]
pub trait IntegrationPort: Send + Sync {
    /// Stable id, lowercase `[a-z][a-z0-9_]*` (for example `"argocd"`). Used as
    /// the settings key and in `ext.<id>.*` tool names for extension-backed
    /// integrations.
    fn id(&self) -> &str;

    /// Which capabilities the integration offers on this cluster; empty when
    /// it is not present. Read-only.
    async fn detect(&self, session: &IntegrationSession) -> OxiResult<Capabilities>;

    /// The sidebar contribution, as data.
    fn sidebar(&self) -> SidebarModel;

    /// The commands the integration adds. Each one also needs a tool (see
    /// [`ToolDef::for_command`](crate::tool::ToolDef::for_command)) unless
    /// privileged.
    fn commands(&self) -> Vec<CommandMeta>;

    /// The agent tools the integration adds (`argo.*` and so on).
    fn tools(&self) -> Vec<Arc<dyn ToolPort>>;

    /// The `@`-mention providers the integration adds.
    fn context_providers(&self) -> Vec<Arc<dyn ContextProviderPort>>;

    /// JSON Schema of the integration's settings, as an object schema. Return
    /// `{"type": "object"}` when it has none.
    fn settings_schema(&self) -> Value;
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxikube_domain::session::ClusterSessionState;

    fn item(id: &str, needs: Capabilities) -> SidebarItem {
        SidebarItem {
            id: id.into(),
            title: id.into(),
            icon: Some("git-branch".into()),
            command: CommandId::new("argo::HardRefresh"),
            needs,
        }
    }

    #[test]
    fn visible_with_drops_unmet_items_and_empty_sections() {
        let model = SidebarModel {
            sections: vec![
                SidebarSection {
                    id: "apps".into(),
                    title: "Applications".into(),
                    icon: None,
                    items: vec![
                        item("list", Capabilities::ARGO),
                        item("sync", Capabilities::ARGO | Capabilities::MUTATE),
                    ],
                },
                SidebarSection {
                    id: "helm".into(),
                    title: "Helm".into(),
                    icon: None,
                    items: vec![item("releases", Capabilities::HELM)],
                },
            ],
        };
        let shown = model.visible_with(Capabilities::ARGO);
        assert_eq!(shown.sections.len(), 1);
        assert_eq!(shown.sections[0].items.len(), 1);
        assert_eq!(shown.sections[0].items[0].id, "list");
        assert!(
            model
                .visible_with(Capabilities::empty())
                .sections
                .is_empty()
        );
        assert_eq!(model.visible_with(Capabilities::all()), model);
    }

    #[test]
    fn session_snapshot_reports_served_kinds_and_connection() {
        let app = Gvk::from_api_version("argoproj.io/v1alpha1", "Application");
        let s = IntegrationSession::new(
            ClusterId::new("kubeconfig", &ContextName::new("kind-oxikube")),
            ContextName::new("kind-oxikube"),
            ClusterSessionState::Ready,
            [app.clone()],
        );
        assert!(s.serves(&app));
        assert!(!s.serves(&Gvk::from_api_version("v1", "Pod")));
        assert!(s.is_connected());
        let down = IntegrationSession {
            state: ClusterSessionState::Disconnected,
            ..s
        };
        assert!(!down.is_connected());
    }

    #[test]
    fn sidebar_serialises_for_the_extension_api() {
        let model = SidebarModel::single(SidebarSection {
            id: "apps".into(),
            title: "Applications".into(),
            icon: None,
            items: vec![item("list", Capabilities::ARGO)],
        });
        let v = serde_json::to_value(&model).unwrap();
        assert_eq!(v["sections"][0]["items"][0]["command"], "argo::HardRefresh");
        assert_eq!(
            v["sections"][0]["items"][0]["needs"],
            serde_json::json!(["argo"])
        );
    }
}
