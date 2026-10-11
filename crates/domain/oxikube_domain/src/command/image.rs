//! [`ImageChange`]: one container's new image, the argument of `workload::SetImage`.

use serde::{Deserialize, Serialize};

/// One container whose image `workload::SetImage` changes: `kubectl set image web=nginx:1.27`.
///
/// `container` names a container or an init container of the workload's pod template; the
/// handler finds out which by reading the live template (container names are unique across both
/// lists), so a caller names no list.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ImageChange {
    /// The container's name in `spec.template.spec`.
    pub container: String,
    /// The image reference to run (`nginx:1.27`, `registry.example/app@sha256:...`).
    pub image: String,
}

impl ImageChange {
    /// A change of `container` to `image`.
    pub fn new(container: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            container: container.into(),
            image: image.into(),
        }
    }
}
