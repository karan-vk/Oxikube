//! The type ids and the claim read the storage handlers and flows share.

use oxikube_domain::ids::Gvk;
use oxikube_domain::view::storage::{PvcSummary, StorageClassSummary};
use oxikube_domain::{OxiError, OxiResult};
use oxikube_ports::ResourceReader;

/// The core `PersistentVolumeClaim` type.
pub(crate) fn claim_gvk() -> Gvk {
    Gvk::new("", "v1", "PersistentVolumeClaim")
}

/// The `storage.k8s.io` `StorageClass` type.
pub(crate) fn class_gvk() -> Gvk {
    Gvk::new("storage.k8s.io", "v1", "StorageClass")
}

/// The live claim `name` in `namespace` and the StorageClass it names. A class that cannot be
/// read is `None` ("unknown", not "forbidden": the server decides).
///
/// # Errors
///
/// The claim's read error, or a validation error when it is not a claim.
pub(crate) async fn read_claim(
    reader: &dyn ResourceReader,
    namespace: Option<&str>,
    name: &str,
) -> OxiResult<(PvcSummary, Option<StorageClassSummary>)> {
    let live = reader.get(&claim_gvk(), namespace, name).await?;
    let claim =
        PvcSummary::from_resource(&live).map_err(|e| OxiError::validation(e.to_string()))?;
    let class = match claim.storage_class.as_deref() {
        Some(class) => reader
            .get_opt(&class_gvk(), None, class)
            .await
            .ok()
            .flatten()
            .and_then(|res| StorageClassSummary::from_resource(&res).ok()),
        None => None,
    };
    Ok((claim, class))
}
