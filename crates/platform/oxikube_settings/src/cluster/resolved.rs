//! [`ClusterSettings`]: the resolved per-cluster settings and how to read and observe them.

use std::ops::Deref;
use std::sync::Arc;

use gpui::{App, Subscription};
use oxikube_domain::ids::ClusterId;
use oxikube_ports::{ClusterPrefs, ClusterPrefsTable, PrometheusOverride};
use serde_json::Value;

use super::content::{ClusterSettingsContent, PrometheusContent};
use crate::settings::{Settings, SettingsLocation};
use crate::store::SettingsStore;

/// The settings of one cluster, resolved field by field from `default.json`, the user's
/// top-level values and the cluster's own `clusters.<id>` block (see
/// [`ClusterSettingsContent`] for the keys).
///
/// Read one cluster's values with [`ClusterSettings::resolve`] (a hash lookup, no parsing),
/// react to changes of one cluster with [`ClusterSettings::observe_cluster`], and hand the
/// whole picture to the app layer with [`ClusterSettings::table`]. The wrapped
/// [`ClusterPrefs`] is plain data the app crate understands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClusterSettings {
    prefs: Arc<ClusterPrefs>,
}

impl Deref for ClusterSettings {
    type Target = ClusterPrefs;

    fn deref(&self) -> &ClusterPrefs {
        &self.prefs
    }
}

impl Settings for ClusterSettings {
    const KEY: Option<&'static str> = None;
    type Content = ClusterSettingsContent;

    fn from_content(content: ClusterSettingsContent) -> Self {
        Self {
            prefs: Arc::new(prefs_from_content(content)),
        }
    }

    /// `read_only` fails closed: when a block does not deserialise (a typo in some other
    /// field), a `read_only` that is anything but plainly off still makes the cluster
    /// read-only, on a first load as much as on a reload. Everything else keeps `base`.
    fn salvage(merged: &Value, base: &Self) -> Option<Self> {
        let wants_read_only = !matches!(
            merged.get("read_only"),
            None | Some(Value::Null | Value::Bool(false))
        );
        (wants_read_only && !base.prefs.read_only).then(|| Self {
            prefs: Arc::new(ClusterPrefs {
                read_only: true,
                ..ClusterPrefs::clone(&base.prefs)
            }),
        })
    }
}

crate::register_settings!(ClusterSettings);

/// Resolves merged content into plain prefs: blank strings are unset, namespace lists are
/// trimmed and deduplicated in order.
fn prefs_from_content(content: ClusterSettingsContent) -> ClusterPrefs {
    let ClusterSettingsContent {
        display_name,
        colour,
        read_only,
        default_namespace,
        terminal_cwd,
        node_shell_image,
        node_shell_pull_secret,
        node_shell,
        prometheus,
        accessible_namespaces,
        exec_interactivity,
        exec_in_read_only,
        watch_budget,
    } = content;
    let mut namespaces: Vec<String> = Vec::new();
    for name in accessible_namespaces.unwrap_or_default() {
        let name = name.trim();
        if !name.is_empty() && !namespaces.iter().any(|seen| seen == name) {
            namespaces.push(name.to_owned());
        }
    }
    ClusterPrefs {
        display_name: non_blank(display_name),
        colour,
        read_only: read_only.unwrap_or(false),
        default_namespace: non_blank(default_namespace),
        terminal_cwd: non_blank(terminal_cwd),
        node_shell_image: non_blank(node_shell_image),
        node_shell_pull_secret: non_blank(node_shell_pull_secret),
        node_shell: node_shell.map(Into::into).unwrap_or_default(),
        prometheus: prometheus.and_then(prometheus_from_content),
        accessible_namespaces: namespaces,
        exec_interactivity: exec_interactivity.unwrap_or_default(),
        exec_in_read_only: exec_in_read_only.unwrap_or(false),
        watch_budget: watch_budget.map(Into::into).unwrap_or_default(),
    }
}

fn prometheus_from_content(content: PrometheusContent) -> Option<PrometheusOverride> {
    let value = PrometheusOverride {
        provider: non_blank(content.provider),
        path: non_blank(content.path),
        url: content.url.map(|url| url.as_str().to_owned()),
        auth: content.auth_secret.map(|name| name.key()),
    };
    (value != PrometheusOverride::default()).then_some(value)
}

pub(super) fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
}

impl ClusterSettings {
    /// The resolved prefs, shareable with the app layer.
    pub fn prefs(&self) -> &Arc<ClusterPrefs> {
        &self.prefs
    }

    /// The settings of `cluster`: its own block merged over the user's top-level values and the
    /// defaults. A cluster with no `clusters.<id>` block reads the global values.
    ///
    /// Panics when the store is missing or [`ClusterSettings`] is not registered (see
    /// [`Settings::get`]).
    #[track_caller]
    pub fn resolve<'a>(cluster: &ClusterId, cx: &'a App) -> &'a ClusterSettings {
        Self::get(Some(SettingsLocation { cluster }), cx)
    }

    fn try_resolve<'a>(cluster: &ClusterId, cx: &'a App) -> Option<&'a ClusterSettings> {
        cx.try_global::<SettingsStore>()?
            .try_get(Some(SettingsLocation { cluster }))
    }

    /// The resolved prefs of every cluster that has overrides, plus the global fallback, as the
    /// lookup index the app layer takes. Keys in `clusters` that are not cluster ids are
    /// skipped (and logged): an id is 16 lowercase hex characters.
    pub fn table(cx: &App) -> ClusterPrefsTable {
        let Some(store) = cx.try_global::<SettingsStore>() else {
            return ClusterPrefsTable::default();
        };
        let default = store
            .try_get::<ClusterSettings>(None)
            .map(|settings| settings.prefs.clone())
            .unwrap_or_default();
        let mut table = ClusterPrefsTable::new(default);
        for (key, settings) in store.cluster_values::<ClusterSettings>() {
            match key.parse::<ClusterId>() {
                Ok(id) => table = table.with_cluster(id, settings.prefs.clone()),
                Err(_) => tracing::warn!(key, "`clusters` key is not a cluster id; ignored"),
            }
        }
        table
    }

    /// Calls `f` with the new value whenever `cluster`'s resolved settings change.
    ///
    /// Edits that leave this cluster's values equal (another cluster's block, an unrelated
    /// setting) do not call it, so a view of one cluster never wakes for another's change.
    pub fn observe_cluster(
        cx: &mut App,
        cluster: ClusterId,
        mut f: impl FnMut(&ClusterSettings, &mut App) + 'static,
    ) -> Subscription {
        let mut last = Self::try_resolve(&cluster, cx).cloned();
        cx.observe_global::<SettingsStore>(move |cx| {
            let Some(now) = Self::try_resolve(&cluster, cx).cloned() else {
                return;
            };
            if last.as_ref() != Some(&now) {
                last = Some(now.clone());
                f(&now, cx);
            }
        })
    }
}
