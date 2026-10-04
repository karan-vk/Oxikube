// Portions of this file are derived from Zed (https://github.com/zed-industries/zed),
// Copyright (c) Zed Industries, Inc. and contributors.
// Zed is licensed under the GNU General Public License v3.0 or later.
// Modifications Copyright (c) Oxikube contributors.
// SPDX-License-Identifier: GPL-3.0-or-later
// Source: crates/settings/src/settings_store.rs @ zed a84689073d296dfd39987bc7dd478e43ef76d83a

//! The [`Settings`] trait, [`SettingsLocation`] and inventory registration.
//!
//! The trait follows Zed's shape (`register`, `get`, `get_global`, `try_get`,
//! `override_global`) with the per-crate content type of Zed's `SettingsKey` era: each
//! feature crate owns a serde "content" struct (`Option` fields, `JsonSchema`) read from its
//! own key of `settings.json`, and resolves it into a runtime struct with
//! [`Settings::from_content`].

use gpui::{App, Context, Subscription, UpdateGlobal as _};
use oxikube_domain::ids::ClusterId;
use schemars::JsonSchema;
use serde::{Serialize, de::DeserializeOwned};

use crate::store::SettingsStore;

/// Where a setting is read for: `None` (in the `get` APIs) means the global value, a
/// location narrows it to one cluster's `clusters.<id>` overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsLocation<'a> {
    /// The cluster whose overrides apply.
    pub cluster: &'a ClusterId,
}

/// Bound for a setting's file content: what one layer of `settings.json` may say about it.
///
/// Use `Option` fields with `#[serde(default, skip_serializing_if = "Option::is_none")]` so a
/// layer can set any subset, and derive `JsonSchema` so the field shows up in
/// `settings.schema.json`. Unknown fields are ignored on read (and reported as diagnostics).
pub trait SettingsContent:
    Default + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

impl<T> SettingsContent for T where
    T: Default + Serialize + DeserializeOwned + JsonSchema + Send + Sync + 'static
{
}

/// A typed setting resolved from the layered settings files.
///
/// The value is read from the merged layers (default.json, then the user's settings.json,
/// then `clusters.<id>` for a cluster location) at [`Settings::KEY`], deserialised into
/// [`Settings::Content`] and resolved with [`Settings::from_content`]. Every field must have
/// a default in `default.json`, so `from_content` may treat a missing value as a bug.
///
/// `PartialEq` is how the store tells a real change from a reload that left the value equal;
/// only real changes wake [`Settings::observe`] observers.
///
/// Register with [`register_settings!`](crate::register_settings) (picked up by
/// [`SettingsStore::new`] through `inventory`) or [`Settings::register`].
pub trait Settings: PartialEq + Send + Sync + Sized + 'static {
    /// Top-level key in `settings.json`, or `None` to read the content from the root object
    /// (its fields are then top-level keys). `"clusters"` is reserved for the cluster layer.
    const KEY: Option<&'static str>;

    /// The serde content one layer provides.
    type Content: SettingsContent;

    /// Resolve the merged content into the runtime value.
    fn from_content(content: Self::Content) -> Self;

    /// Register this setting with the global store (a no-op when already registered).
    fn register(cx: &mut App) {
        SettingsStore::update_global(cx, |store, _| store.register_setting::<Self>());
    }

    /// The value for `location` (the global value for `None`).
    ///
    /// Panics when the setting is not registered: that is a wiring bug, like reading a GPUI
    /// global that was never set.
    #[track_caller]
    fn get<'a>(location: Option<SettingsLocation>, cx: &'a App) -> &'a Self {
        cx.global::<SettingsStore>().get(location)
    }

    /// The global value. Panics when the setting is not registered (see [`Settings::get`]).
    #[track_caller]
    fn get_global(cx: &App) -> &Self {
        cx.global::<SettingsStore>().get(None)
    }

    /// The global value, or `None` when there is no store or the setting is not registered.
    fn try_get(cx: &App) -> Option<&Self> {
        cx.try_global::<SettingsStore>()?.try_get(None)
    }

    /// Replace the global value until the next reload (tests and previews).
    fn override_global(value: Self, cx: &mut App) {
        SettingsStore::update_global(cx, |store, _| store.override_global(value));
    }

    /// Call `f` whenever this setting's resolved value changes (global or any cluster).
    ///
    /// Reloads that leave the value equal do not call it, so unrelated edits to
    /// `settings.json` never wake this observer.
    fn observe(cx: &mut App, mut f: impl FnMut(&mut App) + 'static) -> Subscription {
        let mut seen = cx
            .try_global::<SettingsStore>()
            .map_or(0, |store| store.generation::<Self>());
        cx.observe_global::<SettingsStore>(move |cx| {
            let current = cx.global::<SettingsStore>().generation::<Self>();
            if current != seen {
                seen = current;
                f(cx);
            }
        })
    }

    /// [`Settings::observe`] for an entity: `f` runs with the entity while it is alive.
    fn observe_in<V: 'static>(
        cx: &mut Context<V>,
        mut f: impl FnMut(&mut V, &mut Context<V>) + 'static,
    ) -> Subscription {
        let mut seen = cx
            .try_global::<SettingsStore>()
            .map_or(0, |store| store.generation::<Self>());
        cx.observe_global::<SettingsStore>(move |this, cx| {
            let current = cx.global::<SettingsStore>().generation::<Self>();
            if current != seen {
                seen = current;
                f(this, cx);
            }
        })
    }
}

/// A setting type collected through `inventory` and registered by [`SettingsStore::new`].
///
/// Build it with [`register_settings!`](crate::register_settings).
pub struct RegisteredSetting {
    register: fn(&mut SettingsStore),
}

impl RegisteredSetting {
    /// The registration record for `T`.
    pub const fn of<T: Settings>() -> Self {
        Self {
            register: SettingsStore::register_setting::<T>,
        }
    }

    pub(crate) fn register_into(&self, store: &mut SettingsStore) {
        (self.register)(store);
    }
}

inventory::collect!(RegisteredSetting);

/// Register one or more [`Settings`] types so every [`SettingsStore`] created afterwards
/// loads them (Zed's `#[derive(RegisterSetting)]`, as a declarative macro).
///
/// ```ignore
/// oxikube_settings::register_settings!(TerminalSettings, EditorSettings);
/// ```
///
/// Registration relies on the linker keeping the crate: a crate nothing references is not
/// linked and its settings stay unregistered. Crates expose `init(cx)`, which the binary
/// calls, so this holds in the app; tests can also call [`Settings::register`].
#[macro_export]
macro_rules! register_settings {
    ($($setting:ty),+ $(,)?) => {
        $(
            $crate::private::inventory::submit! {
                $crate::RegisteredSetting::of::<$setting>()
            }
        )+
    };
}
