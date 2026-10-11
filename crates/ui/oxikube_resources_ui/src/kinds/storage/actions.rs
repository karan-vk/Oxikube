//! The storage row actions: "Resize…" on claims and "Set as Default" on storage classes.

use oxikube_app::RowActionSpec;
use oxikube_app::actions::KindFilter;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::ids::Gvk;

/// Whether `gvk` is the core PersistentVolumeClaim.
pub fn is_claim(gvk: &Gvk) -> bool {
    gvk.group.is_empty() && &*gvk.kind == "PersistentVolumeClaim"
}

/// Whether `gvk` is a `storage.k8s.io` StorageClass.
pub fn is_storage_class(gvk: &Gvk) -> bool {
    &*gvk.group == "storage.k8s.io" && &*gvk.kind == "StorageClass"
}

/// The specs the app adds to its [`RowActionRegistry`](oxikube_app::RowActionRegistry): for
/// PersistentVolumeClaim rows "Resize…" (`pvc::Resize`, which opens the resize dialog), for
/// StorageClass rows "Set as Default" (`storageclass::SetDefault`, which opens its dialog).
/// Both mutate, so a read-only cluster shows them disabled with the reason; the guard refuses
/// them for every initiator regardless.
///
/// The commands here carry placeholder arguments (`size` empty, `clear_previous` on): the table
/// never dispatches them from the menu, it opens the dialog, and the dialog sends the real one.
pub fn storage_row_actions() -> [RowActionSpec; 2] {
    [
        RowActionSpec::new(CommandId::PVC_RESIZE, |target| Command::PvcResize {
            target: target.clone(),
            size: String::new(),
            from: None,
        })
        .label("Resize…")
        .kinds(KindFilter::Matching(|kind| is_claim(&kind.gvk)))
        .order(100),
        RowActionSpec::new(CommandId::STORAGECLASS_SET_DEFAULT, |target| {
            Command::StorageClassSetDefault {
                target: target.clone(),
                clear_previous: true,
            }
        })
        .label("Set as Default")
        .kinds(KindFilter::Matching(|kind| is_storage_class(&kind.gvk)))
        .order(100),
    ]
}
