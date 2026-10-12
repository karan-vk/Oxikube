//! The values of a pod's own fields: `fieldRef` and `resourceFieldRef` environment variables.

use crate::resource::Resource;
use crate::view::{arr_of, str_of, sub};

/// The value of a `fieldRef` path of `pod`: `metadata.name`, `metadata.namespace`, `metadata.uid`,
/// `metadata.labels['k']`, `metadata.annotations['k']`, `spec.nodeName`, `spec.serviceAccountName`,
/// `status.hostIP`, `status.podIP` and `status.podIPs` (comma separated).
pub(super) fn field_value(pod: &Resource, path: &str) -> Option<String> {
    let json = pod.json();
    for (prefix, key) in [
        ("metadata.labels['", "labels"),
        ("metadata.annotations['", "annotations"),
    ] {
        if let Some(rest) = path.strip_prefix(prefix) {
            let label = rest.strip_suffix("']")?;
            return str_of(sub(sub(json, "metadata"), key), label).map(str::to_owned);
        }
    }
    if path == "status.podIPs" {
        let ips: Vec<&str> = arr_of(sub(json, "status"), "podIPs")
            .iter()
            .filter_map(|ip| str_of(ip, "ip"))
            .collect();
        return (!ips.is_empty()).then(|| ips.join(","));
    }
    let mut at = json;
    for segment in path.split('.') {
        at = at.get(segment)?;
    }
    at.as_str().map(str::to_owned)
}

/// The request or limit `resource` (`limits.cpu`) of the container `container`, as written in
/// the spec.
pub(super) fn resource_value(pod: &Resource, container: &str, resource: &str) -> Option<String> {
    let (section, name) = resource.split_once('.')?;
    let spec = sub(pod.json(), "spec");
    ["containers", "initContainers"]
        .into_iter()
        .flat_map(|list| arr_of(spec, list).iter())
        .find(|c| str_of(*c, "name") == Some(container))
        .and_then(|c| str_of(sub(sub(c, "resources"), section), name))
        .map(str::to_owned)
}
