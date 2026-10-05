//! [`ItemRegistry`]: rebuilds items from their serialised state.
//!
//! Reopening a closed tab keeps only a descriptor (kind + state), never the live entity, so the
//! item has to be rebuilt; layout persistence (E05-S05) restores tabs the same way. A feature
//! crate registers a builder for each item kind from its `init(cx)`.

use std::{collections::HashMap, rc::Rc};

use gpui::{App, Entity, Global, Window};

use super::{Item, ItemHandle};

/// Builds an item from the state its [`Item::serialize`] produced. `None`: the state is no longer
/// usable (the resource is gone, the format is outdated), so nothing is opened.
pub type ItemBuilder =
    Rc<dyn Fn(&serde_json::Value, &mut Window, &mut App) -> Option<Box<dyn ItemHandle>>>;

/// The item builders, keyed by [`Item::serialized_kind`]. An app-wide global.
#[derive(Default)]
pub struct ItemRegistry {
    builders: HashMap<&'static str, ItemBuilder>,
}

impl Global for ItemRegistry {}

impl ItemRegistry {
    /// Whether a builder is registered for `kind`.
    pub fn is_registered(cx: &App, kind: &str) -> bool {
        cx.try_global::<Self>()
            .is_some_and(|registry| registry.builders.contains_key(kind))
    }

    /// Rebuilds an item of `kind` from `state`. `None` when no builder is registered for the kind
    /// or the builder declines the state.
    pub fn build(
        kind: &str,
        state: &serde_json::Value,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Box<dyn ItemHandle>> {
        // Cloned out so the builder may itself read or write globals.
        let builder = cx.try_global::<Self>()?.builders.get(kind)?.clone();
        builder(state, window, cx)
    }
}

/// Registers how to rebuild items of type `T` from their serialised state. Call it from the
/// owning crate's `init(cx)`. A second registration for the same kind replaces the first.
///
/// Does nothing for an item type without [`Item::serialized_kind`]: such items cannot be
/// rebuilt.
pub fn register_item<T: Item>(
    cx: &mut App,
    build: impl Fn(&serde_json::Value, &mut Window, &mut App) -> Option<Entity<T>> + 'static,
) {
    let Some(kind) = T::serialized_kind() else {
        debug_assert!(
            false,
            "register_item for an item type without a serialized_kind"
        );
        return;
    };
    let builder: ItemBuilder = Rc::new(move |state, window, cx| {
        build(state, window, cx).map(|item| Box::new(item) as Box<dyn ItemHandle>)
    });
    cx.default_global::<ItemRegistry>()
        .builders
        .insert(kind, builder);
}
