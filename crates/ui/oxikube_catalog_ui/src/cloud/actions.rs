//! The Cloud section's UI-local actions: what its keys are bound to.
//!
//! Each stands for the `Command` of the same name (`cloud::Refresh`, `cloud::Cancel`) and does
//! nothing but send it through the view's backend, so the key, the buttons, the palette and an
//! agent all run one behaviour. The default bindings live in the per-OS keymap files of
//! `oxikube_assets`, in the section for the `CatalogCloud` key context
//! (`oxikube_keymap::contexts::CATALOG_CLOUD`), so users rebind them in `keymap.json`.

use gpui::actions;

actions!(
    cloud,
    [
        /// Refresh every provider now, even when the cache is fresh (`cloud::Refresh`).
        Refresh,
        /// Stop the running discovery (`cloud::Cancel`, the Escape key).
        Cancel,
    ]
);
