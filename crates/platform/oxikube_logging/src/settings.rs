//! The `log` setting: the log filter, hot reloaded.
//!
//! ```json
//! "log": { "filter": "info,oxikube_kube=debug" }
//! ```
//!
//! `filter` is `RUST_LOG` syntax. `default.json` carries [`DEFAULT_DIRECTIVES`] (a test keeps the
//! two equal). Logging starts before the settings store exists (it is the first thing `main`
//! does, so a failure in settings is logged), with the shipped default; [`follow`] then applies
//! the setting and every later edit of it to the running subscriber through its [`LogHandle`].
//! A `RUST_LOG` that was valid at start-up wins over the setting for that run.

use gpui::{App, Subscription};
use oxikube_settings::Settings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{DEFAULT_DIRECTIVES, LogHandle, SetOutcome};

/// What one settings layer says about logging.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct LogSettingsContent {
    /// Log filter in `RUST_LOG` syntax, for example `info,oxikube_kube=debug`. Keep the HTTP
    /// stack (hyper, h2, tower, rustls) at `warn` or quieter: at debug and trace they log request
    /// headers (the writer redacts them, but they are noise). A `RUST_LOG` set at launch takes
    /// precedence for that run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
}

/// The resolved `log` setting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSettings {
    /// Filter directives.
    pub filter: String,
}

impl Settings for LogSettings {
    const KEY: Option<&'static str> = Some("log");
    type Content = LogSettingsContent;

    fn from_content(content: LogSettingsContent) -> Self {
        Self {
            filter: content
                .filter
                .unwrap_or_else(|| DEFAULT_DIRECTIVES.to_owned()),
        }
    }
}

oxikube_settings::register_settings!(LogSettings);

/// Applies the `log.filter` setting to `handle` now and whenever it changes. An invalid filter is
/// logged as a warning and the previous one stays. Keep (or `.detach()`) the subscription.
pub fn follow(cx: &mut App, handle: LogHandle) -> Subscription {
    apply(cx, &handle);
    LogSettings::observe(cx, move |cx| apply(cx, &handle))
}

fn apply(cx: &App, handle: &LogHandle) {
    let Some(settings) = LogSettings::try_get(cx) else {
        return;
    };
    if settings.filter == handle.directives() {
        return;
    }
    match handle.set_directives(&settings.filter) {
        Ok(SetOutcome::Applied) => {
            tracing::info!(filter = %settings.filter, "log filter changed");
        }
        Ok(SetOutcome::PinnedByRustLog) => {
            tracing::info!("log.filter ignored: RUST_LOG is set for this run");
        }
        Err(err) => tracing::warn!(%err, "keeping the previous log filter"),
    }
}

#[cfg(test)]
mod tests;
