//! `service::ShowPods`: from a Service to a table of the pods it selects.

use oxikube_domain::command::Command;
use oxikube_domain::ids::ClusterId;
use oxikube_domain::session::WatchScope;
use oxikube_domain::view::{NoSelector, ServiceSummary};

use super::ServiceBackends;

/// The commands that show the pods of `service` in `cluster`, in order: select the Service's
/// namespace when `selection` does not include it, open the Pods table, and filter it with the
/// Service's selector in the filter bar's grammar (`-l app=web,tier=front`), which the table
/// sends to the server.
///
/// # Errors
///
/// [`NoSelector`] when the Service selects no pods (ExternalName, or no `spec.selector`): there
/// is no table to open, and the reason is for the user.
pub fn plan_show_pods(
    cluster: &ClusterId,
    service: &ServiceSummary,
    selection: &WatchScope,
) -> Result<Vec<Command>, NoSelector> {
    let selector = service.pod_selector().ok_or_else(|| {
        service
            .no_selector()
            .expect("a service without a pod selector has a reason")
    })?;
    let mut commands = Vec::with_capacity(3);
    if let (Some(namespace), WatchScope::Namespaces(selected)) = (&service.namespace, selection)
        && !selected.iter().any(|n| n.as_str() == &**namespace)
    {
        commands.push(Command::NamespaceSelect {
            cluster: cluster.clone(),
            namespaces: vec![namespace.to_string()],
        });
    }
    let pods = ServiceBackends::pod_gvk();
    commands.push(Command::ResourceOpenList {
        cluster: cluster.clone(),
        gvk: pods.clone(),
    });
    commands.push(Command::TableSetFilter {
        cluster: cluster.clone(),
        gvk: pods,
        text: format!("-l {selector}"),
    });
    Ok(commands)
}
