//! The raw event source of one watch: `kube_runtime::watcher` with its `watcher::Config`
//! and backon backoff.

use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use kube::Api;
use kube::core::DynamicObject;
use kube::runtime::WatchStreamExt;
use kube::runtime::utils::ResetTimerBackoff;
use kube::runtime::watcher::{self, ExponentialBackoff, InitialListStrategy};
use oxikube_ports::WatchOptions;

use super::config::FeedConfig;

/// After this long without needing a retry, the backoff starts again from its minimum.
const BACKOFF_RESET: Duration = Duration::from_secs(120);

/// The watcher's events, with backoff applied after every error.
pub(super) type EventStream = BoxStream<'static, watcher::Result<watcher::Event<DynamicObject>>>;

/// The `watcher::Config` for `options`: selectors, initial-list page size, server-side
/// timeout, bookmarks on, and a streaming list when `streaming`.
pub(super) fn watcher_config(
    options: &WatchOptions,
    config: &FeedConfig,
    streaming: bool,
) -> watcher::Config {
    let nonempty = |s: &Option<String>| s.clone().filter(|v| !v.is_empty());
    let page_size = options
        .page_size
        .filter(|&n| n > 0)
        .unwrap_or(config.page_size);
    let mut wc = watcher::Config::default()
        .page_size(page_size)
        .timeout(config.watch_timeout_secs);
    wc.label_selector = nonempty(&options.label_selector);
    wc.field_selector = nonempty(&options.field_selector);
    if streaming {
        wc = wc.streaming_lists();
    }
    wc
}

/// Whether `wc` asks for a streaming list.
pub(super) fn is_streaming(wc: &watcher::Config) -> bool {
    wc.initial_list_strategy == InitialListStrategy::StreamingList
}

/// Exponential backoff (backon) from `backoff_min` to `backoff_max`, doubling, optionally
/// jittered, reset after two quiet minutes. kube's `StreamBackoff` also resets it on every
/// successful event.
fn backoff(config: &FeedConfig) -> ResetTimerBackoff<ExponentialBackoff> {
    let builder = backon::ExponentialBuilder::default()
        .with_min_delay(config.backoff_min)
        .with_max_delay(config.backoff_max)
        .with_factor(2.0)
        .without_max_times();
    let builder = if config.backoff_jitter {
        builder.with_jitter()
    } else {
        builder
    };
    ResetTimerBackoff::new(ExponentialBackoff::from(builder), BACKOFF_RESET)
}

/// Opens the watcher. Nothing is requested until the stream is first polled.
pub(super) fn events(
    api: Api<DynamicObject>,
    wc: watcher::Config,
    config: &FeedConfig,
) -> EventStream {
    watcher::watcher(api, wc).backoff(backoff(config)).boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_map_onto_the_watcher_config() {
        let options = WatchOptions::default()
            .labels("app=web")
            .fields("")
            .page_size(50);
        let wc = watcher_config(&options, &FeedConfig::default(), false);
        assert_eq!(wc.label_selector.as_deref(), Some("app=web"));
        assert_eq!(wc.field_selector, None, "empty selector means none");
        assert_eq!(wc.page_size, Some(50));
        assert_eq!(wc.timeout, Some(290));
        assert!(wc.bookmarks);
        assert!(!is_streaming(&wc));

        let wc = watcher_config(&WatchOptions::default(), &FeedConfig::default(), true);
        assert_eq!(wc.page_size, Some(500));
        assert!(is_streaming(&wc));
    }

    #[test]
    fn backoff_doubles_up_to_the_cap() {
        let config = FeedConfig {
            backoff_min: Duration::from_millis(100),
            backoff_max: Duration::from_millis(350),
            backoff_jitter: false,
            ..FeedConfig::default()
        };
        let delays: Vec<_> = backoff(&config).take(4).collect();
        let ms: Vec<_> = delays.iter().map(Duration::as_millis).collect();
        assert_eq!(ms, vec![100, 200, 350, 350]);
    }
}
