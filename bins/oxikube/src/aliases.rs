//! The `:` jump bar's alias tables, kept current (E11-S04).
//!
//! [`start`] runs with the feature `init`s (see `startup::features`) and does two things, neither
//! on the first frame's path:
//!
//! - [`AliasRegistry::follow`] on the session manager: a cluster that connects gets the aliases
//!   its discovery serves (plural, singular, short names, Kind, CRDs included), a CRD change
//!   updates only the groups it touches, a disconnect drops them. It runs on the Tokio runtime.
//! - the user's `aliases.json` next to `settings.json`: read and watched on the background
//!   executor, every save pushed into the registry from the watcher's thread. Invalid entries
//!   are logged with their line and skipped; the rest load. In tests the file is read once and
//!   never watched ([`ConfigSource::Dir`]); with in-memory config nothing is read.
//!
//! The registry itself is `AppState::services().aliases`: the jump bar (E11-S05) asks it for the
//! table of the active cluster and resolves each word through that.

use gpui::{App, AppContext as _, BorrowAppContext as _, Global};
use oxikube_app::{AliasFollow, AliasRegistry};
use oxikube_settings::aliases::{LoadedAliases, UserAliasesFile, user_aliases_path};

use crate::app_state::AppState;
use crate::startup::ConfigSource;

/// What keeps the alias machinery alive for the life of the app.
pub struct AliasWiring {
    _follow: Option<AliasFollow>,
    file: Option<UserAliasesFile>,
}

impl AliasWiring {
    /// The user's file, once it has been opened (it opens in the background).
    pub fn file(&self) -> Option<&UserAliasesFile> {
        self.file.as_ref()
    }
}

impl Global for AliasWiring {}

/// Where `init` was told to look for config, kept so the feature inits (which take only the
/// `App`) can honour it.
#[derive(Clone)]
pub(crate) struct ConfigSourceGlobal(pub ConfigSource);

impl Global for ConfigSourceGlobal {}

/// Starts following the clusters and the user's `aliases.json`. Does nothing without an
/// [`AppState`].
pub fn start(cx: &mut App) {
    let Some(state) = AppState::try_global(cx) else {
        return;
    };
    let registry = state.services().aliases.clone();
    let follow = oxikube_runtime::handle(cx)
        .map(|runtime| registry.follow(&state.services().sessions, &runtime));
    cx.set_global(AliasWiring {
        _follow: follow,
        file: None,
    });

    let path_and_watch = match cx.try_global::<ConfigSourceGlobal>().map(|g| g.0.clone()) {
        Some(ConfigSource::UserDir) => {
            oxikube_settings::paths::config_dir().map(|dir| (user_aliases_path(&dir), true))
        }
        Some(ConfigSource::Dir(dir)) => Some((user_aliases_path(&dir), false)),
        Some(ConfigSource::Memory) | None => None,
    };
    let Some((path, watch)) = path_and_watch else {
        return;
    };
    cx.spawn(async move |cx| {
        let opened = cx
            .background_spawn(async move {
                UserAliasesFile::open(path, watch, move |loaded| push(&registry, loaded))
            })
            .await;
        match opened {
            Ok(file) => {
                cx.update(|cx| cx.update_global::<AliasWiring, _>(|wiring, _| wiring.file = Some(file)));
            }
            Err(error) => {
                tracing::warn!(%error, "aliases.json could not be read: only the built-in aliases are used");
            }
        }
    })
    .detach();
}

/// Hands the parsed file to the registry and says what is wrong with it. Runs on the watcher's
/// thread (and once on the background executor at start-up): it only takes short locks.
fn push(registry: &AliasRegistry, loaded: &LoadedAliases) {
    for diagnostic in &loaded.diagnostics {
        tracing::warn!(%diagnostic, "aliases.json");
    }
    let aliases = loaded
        .aliases
        .iter()
        .map(|alias| (alias.name.clone(), alias.target.clone()))
        .collect();
    for conflict in registry.set_user_aliases(aliases) {
        tracing::info!(
            alias = %conflict.name,
            target = %conflict.winner.target,
            hides = %conflict.others.iter().map(|e| e.target.to_string()).collect::<Vec<_>>().join(", "),
            "an alias in aliases.json hides a built-in one"
        );
    }
    tracing::debug!(count = loaded.aliases.len(), "user aliases loaded");
}

#[cfg(test)]
mod tests;
