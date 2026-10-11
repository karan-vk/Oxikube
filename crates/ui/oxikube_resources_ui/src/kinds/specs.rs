//! [`KindSpecs`]: the registered kind specs, a GPUI global.

use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, Global};
use oxikube_app::{ColumnProvider, GroupKind, KindMeta, KindRegistry, RegisterKindError};
use oxikube_domain::ids::Gvk;

use super::detail::DetailRenderer;
use super::sidebar;
use super::spec::KindSpec;

/// The registered kinds: the app's [`KindRegistry`] and each kind's [`DetailRenderer`]. Use the
/// associated functions with the `App`; reading needs no registration first (an empty registry
/// answers the generic spec).
#[derive(Default)]
pub struct KindSpecs {
    registry: KindRegistry,
    details: HashMap<GroupKind, DetailRenderer>,
}

impl Global for KindSpecs {}

impl KindSpecs {
    /// Registers `spec` and publishes the sidebar again.
    ///
    /// # Errors
    ///
    /// [`RegisterKindError`] when the kind is registered already (the first spec stays), lists
    /// an undeclared command, or has no kind.
    pub fn register(cx: &mut App, spec: KindSpec) -> Result<(), RegisterKindError> {
        Self::register_all(cx, [spec])
    }

    /// Registers each of `specs` and publishes the sidebar once. A spec that is refused is logged
    /// and skipped; the others register.
    ///
    /// # Errors
    ///
    /// The first [`RegisterKindError`].
    pub fn register_all(
        cx: &mut App,
        specs: impl IntoIterator<Item = KindSpec>,
    ) -> Result<(), RegisterKindError> {
        let mut first = None;
        {
            let this = cx.default_global::<KindSpecs>();
            for KindSpec { meta, detail } in specs {
                let key = meta.group_kind();
                match this.registry.register(meta) {
                    Ok(()) => {
                        if !detail.is_generic() {
                            this.details.insert(key, detail);
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "a kind spec was not registered");
                        first.get_or_insert(error);
                    }
                }
            }
        }
        sidebar::publish(cx);
        first.map_or(Ok(()), Err)
    }

    /// The registry (empty when nothing registered).
    pub fn registry(cx: &App) -> KindRegistry {
        cx.try_global::<KindSpecs>()
            .map(|specs| specs.registry.clone())
            .unwrap_or_default()
    }

    /// The spec of `gvk`: the registered one, else the generic one ([`KindMeta::is_generic`]).
    pub fn kind_for(cx: &App, gvk: &Gvk) -> Arc<KindMeta> {
        match cx.try_global::<KindSpecs>() {
            Some(specs) => specs.registry.kind_for(gvk).clone(),
            None => Arc::new(KindMeta::generic()),
        }
    }

    /// The kind's own column provider, `None` when it uses the shared one (or is not registered).
    pub fn columns_for(cx: &App, gvk: &Gvk) -> Option<Arc<dyn ColumnProvider>> {
        cx.try_global::<KindSpecs>()?
            .registry
            .get(gvk)?
            .columns
            .provider()
            .cloned()
    }

    /// How the detail of `gvk` is drawn: the kind's renderer, else the generic detail.
    pub fn detail_for(cx: &App, gvk: &Gvk) -> DetailRenderer {
        cx.try_global::<KindSpecs>()
            .and_then(|specs| specs.details.get(&GroupKind::of(gvk)).cloned())
            .unwrap_or_default()
    }

    /// Runs `f` over the registry without cloning it.
    pub(super) fn read<R>(cx: &App, f: impl FnOnce(&KindRegistry) -> R) -> Option<R> {
        cx.try_global::<KindSpecs>().map(|specs| f(&specs.registry))
    }
}
