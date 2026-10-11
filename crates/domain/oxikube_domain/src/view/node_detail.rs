//! [`NodeDetail`]: what the node detail shows besides the [`NodeSummary`]: capacity next to
//! allocatable, system info, addresses and the images the node has pulled.

use std::sync::Arc;

use super::{arc_of, arr_of, sub};
use crate::quantity::Quantity;
use crate::resource::Resource;

/// One resource of `status.capacity` and `status.allocatable`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityRow {
    /// The resource name: `cpu`, `memory`, `pods`, `ephemeral-storage`, `hugepages-2Mi`, ...
    pub name: Arc<str>,
    /// `status.capacity[name]`, as the API wrote it.
    pub capacity: Option<Arc<str>>,
    /// `status.allocatable[name]`, as the API wrote it.
    pub allocatable: Option<Arc<str>>,
}

/// One image in `status.images`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeImage {
    /// The first name (usually the tagged reference); the rest are in [`names`](Self::names).
    pub name: Arc<str>,
    /// Every name of the image (tags and digests).
    pub names: Vec<Arc<str>>,
    /// `sizeBytes`.
    pub size_bytes: Option<u64>,
}

impl NodeImage {
    /// The size as `123.4 MiB`, empty when unknown.
    pub fn size_text(&self) -> String {
        self.size_bytes
            .and_then(|b| i64::try_from(b).ok())
            .map(|b| Quantity::from_value(b).human_bytes())
            .unwrap_or_default()
    }
}

/// The parts of a Node the detail panel shows beyond [`NodeSummary`](super::NodeSummary).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NodeDetail {
    /// Capacity and allocatable per resource, `cpu`, `memory`, `pods`, `ephemeral-storage`
    /// first and the rest (hugepages, extended resources) sorted by name.
    pub capacity: Vec<CapacityRow>,
    /// `status.nodeInfo` as label and value pairs, in `kubectl describe node` order.
    pub system: Vec<(&'static str, Arc<str>)>,
    /// `status.addresses` as type and address.
    pub addresses: Vec<(Arc<str>, Arc<str>)>,
    /// `status.images`, largest first.
    pub images: Vec<NodeImage>,
    /// `spec.providerID`.
    pub provider_id: Option<Arc<str>>,
    /// `spec.podCIDR`.
    pub pod_cidr: Option<Arc<str>>,
}

/// The `nodeInfo` fields shown, with their labels.
const SYSTEM_INFO: [(&str, &str); 9] = [
    ("Machine ID", "machineID"),
    ("System UUID", "systemUUID"),
    ("Boot ID", "bootID"),
    ("Kernel version", "kernelVersion"),
    ("OS image", "osImage"),
    ("Operating system", "operatingSystem"),
    ("Architecture", "architecture"),
    ("Container runtime", "containerRuntimeVersion"),
    ("Kubelet version", "kubeletVersion"),
];

/// Resources shown first, in this order.
const FIRST: [&str; 4] = ["cpu", "memory", "pods", "ephemeral-storage"];

impl NodeDetail {
    /// Reads the detail of a Node; missing fields give empty parts.
    pub fn from_resource(res: &Resource) -> Self {
        let status = sub(res.json(), "status");
        let spec = sub(res.json(), "spec");
        let capacity = sub(status, "capacity");
        let allocatable = sub(status, "allocatable");

        let mut names: Vec<&str> = Vec::new();
        for section in [capacity, allocatable] {
            if let Some(object) = section.as_object() {
                for key in object.keys() {
                    if !names.contains(&key) {
                        names.push(key);
                    }
                }
            }
        }
        names.sort_by_key(|n| (FIRST.iter().position(|f| f == n).unwrap_or(FIRST.len()), *n));
        let capacity_rows = names
            .into_iter()
            .map(|name| CapacityRow {
                name: Arc::from(name),
                capacity: arc_of(capacity, name),
                allocatable: arc_of(allocatable, name),
            })
            .collect();

        let info = sub(status, "nodeInfo");
        let system = SYSTEM_INFO
            .iter()
            .filter_map(|(label, key)| Some((*label, arc_of(info, key)?)))
            .collect();
        let addresses = arr_of(status, "addresses")
            .iter()
            .filter_map(|a| Some((arc_of(a, "type")?, arc_of(a, "address")?)))
            .collect();
        let mut images: Vec<NodeImage> = arr_of(status, "images")
            .iter()
            .filter_map(|image| {
                let names: Vec<Arc<str>> = arr_of(image, "names")
                    .iter()
                    .filter_map(|n| n.as_str().map(Arc::from))
                    .collect();
                // The tagged reference reads better than the digest; names list the digest first
                // on some runtimes.
                let name = names
                    .iter()
                    .find(|n| !n.contains("@sha256:"))
                    .or_else(|| names.first())?
                    .clone();
                Some(NodeImage {
                    name,
                    names,
                    size_bytes: image.get("sizeBytes").and_then(|v| v.as_u64()),
                })
            })
            .collect();
        images.sort_by(|a, b| {
            b.size_bytes
                .cmp(&a.size_bytes)
                .then_with(|| a.name.cmp(&b.name))
        });

        Self {
            capacity: capacity_rows,
            system,
            addresses,
            images,
            provider_id: arc_of(spec, "providerID"),
            pod_cidr: arc_of(spec, "podCIDR"),
        }
    }
}
