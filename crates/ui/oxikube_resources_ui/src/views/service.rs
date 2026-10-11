//! `service::ShowPods` (E12-S13): from a Service to a table of the pods it selects.
//!
//! The Service is read once on the Tokio bridge (the command may come from the palette or an
//! agent, which have the reference and nothing else). [`plan_show_pods`] turns it into the same
//! commands a jump line sends (`namespace::Select` when the namespace is not selected,
//! `resource::OpenList` for Pods, `table::SetFilter -l <selector>`), and they go through the
//! bus, so the table opens with the selector sent to the server. A Service that selects nothing
//! says why in a toast.

use gpui::{Context, Window};
use oxikube_app::plan_show_pods;
use oxikube_domain::OxiError;
use oxikube_domain::ids::{ResourceRef, Scope};
use oxikube_domain::view::ServiceSummary;
use oxikube_runtime::spawn_kube;
use oxikube_workspace::Toast;

use super::controller::ResourceViews;

impl ResourceViews {
    /// Opens the Pods table filtered by the selector of the Service `target`. See the module
    /// docs.
    pub(super) fn show_service_pods(
        &mut self,
        target: &ResourceRef,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cluster = target.cluster.clone();
        let Some(session) = self.deps.table.sessions.get(&cluster) else {
            return;
        };
        let Some(reader) = session.resources() else {
            self.toast(&cluster, Toast::info("The cluster is not connected."), cx);
            return;
        };
        let selection = session.watch_scope(Scope::Namespaced);
        let (gvk, namespace, name) = (
            target.gvk.clone(),
            target.namespace.clone(),
            target.name.clone(),
        );
        let read = spawn_kube(cx, async move {
            reader.get(&gvk, namespace.as_deref(), &name).await
        });
        let name = target.name.clone();
        self.service_task = Some(cx.spawn_in(window, async move |this, cx| {
            let service = match read.await {
                Ok(Ok(resource)) => ServiceSummary::from_resource(&resource)
                    .map_err(|error| OxiError::validation(error.to_string())),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |views, cx| {
                let plan = service.map(|service| plan_show_pods(&cluster, &service, &selection));
                match plan {
                    Ok(Ok(commands)) => {
                        for command in commands {
                            views.deps.table.dispatcher.dispatch(command, cx);
                        }
                    }
                    Ok(Err(reason)) => {
                        views.toast(&cluster, Toast::info(reason.explanation()), cx);
                    }
                    Err(error) => {
                        tracing::warn!(%error, %name, "showing the pods of a service failed");
                        let text = format!("Could not read {name}: {}", error.message());
                        views.toast(&cluster, Toast::error(text), cx);
                    }
                }
            })
            .ok();
        }));
    }
}
