//! Writing a cluster's block back to the user's `settings.json`.
//!
//! In-app toggles (the read-only switch, a colour picker) edit the file instead of keeping a
//! second copy of the value: the edit goes through [`update_user_settings`], keeps the user's
//! comments and formatting, and flows back through hot reload to the open session.

use gpui::{App, Task};
use oxikube_domain::OxiResult;
use oxikube_domain::ids::ClusterId;

use super::content::ClusterSettingsContent;
use super::resolved::ClusterSettings;
use crate::global::update_user_settings;

impl ClusterSettings {
    /// Edits `clusters.<cluster>` in the user's `settings.json`.
    ///
    /// Setting a field to `None` removes it from the block. When the cluster has no block yet
    /// and `name_hint` is given (the context name), the new block also gets
    /// `"display_name": <hint>` so the user can recognise the opaque id in the file. A block
    /// that exists is never given a name the user did not choose.
    ///
    /// The file is written on the background executor and applied to the store at once; the
    /// task resolves when that is done. Fails (without touching the file) when
    /// `settings.json` does not parse or the cluster's block has a type error.
    pub fn update_cluster(
        cx: &mut App,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        update: impl FnOnce(&mut ClusterSettingsContent) + Send + 'static,
    ) -> Task<OxiResult<()>> {
        let name_hint = name_hint.map(str::to_owned);
        update_user_settings::<ClusterSettings>(cx, Some(cluster.clone()), move |content| {
            let is_new = *content == ClusterSettingsContent::default();
            update(content);
            if is_new
                && content.display_name.is_none()
                && let Some(name) = name_hint
                && *content != ClusterSettingsContent::default()
            {
                content.display_name = Some(name);
            }
        })
    }

    /// Sets `clusters.<cluster>.read_only`. See [`ClusterSettings::update_cluster`].
    ///
    /// Writes `true` and `false` explicitly: a cluster that is writable while the top-level
    /// default is read-only must say so.
    pub fn set_read_only(
        cx: &mut App,
        cluster: &ClusterId,
        name_hint: Option<&str>,
        read_only: bool,
    ) -> Task<OxiResult<()>> {
        Self::update_cluster(cx, cluster, name_hint, move |content| {
            content.read_only = Some(read_only);
        })
    }
}
