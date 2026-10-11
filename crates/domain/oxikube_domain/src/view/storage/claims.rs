//! [`pod_claims`]: which PersistentVolumeClaims a Pod mounts.

use std::sync::Arc;

use super::{arr_of, str_of, sub};
use crate::resource::Resource;

/// The names of the claims `pod` mounts, in volume order, without repeats: every
/// `persistentVolumeClaim.claimName`, and for a generic ephemeral volume the claim the
/// ephemeral-volume controller creates for it, `<pod>-<volume>`. Empty for anything that is not
/// a Pod with volumes.
///
/// Reads `spec.volumes` in place and allocates only the returned names, so it is cheap enough
/// to run once per Pod delta.
pub fn pod_claims(pod: &Resource) -> Vec<Arc<str>> {
    let mut claims: Vec<Arc<str>> = Vec::new();
    for volume in arr_of(sub(pod.json(), "spec"), "volumes").iter() {
        let claim = match str_of(sub(volume, "persistentVolumeClaim"), "claimName") {
            Some(name) => Arc::from(name),
            None if volume.get("ephemeral").is_some() => {
                let Some(volume_name) = str_of(volume, "name") else {
                    continue;
                };
                Arc::from(format!("{}-{volume_name}", pod.meta.name))
            }
            None => continue,
        };
        if !claims.contains(&claim) {
            claims.push(claim);
        }
    }
    claims
}

/// Whether `pod` can still use its volumes: it has not run to completion (`Succeeded`) or
/// failed for good (`Failed`). A finished pod no longer holds a claim, and `kubectl describe pvc`
/// leaves it out of "Used By".
pub fn pod_is_active(pod: &Resource) -> bool {
    !matches!(
        str_of(sub(pod.json(), "status"), "phase"),
        Some("Succeeded" | "Failed")
    )
}
