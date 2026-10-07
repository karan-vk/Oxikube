//! The multi-pod side of the log view (E08-S04): opening a Deployment, StatefulSet, DaemonSet,
//! ReplicaSet, Job or Service as one merged log.
//!
//! | File | Holds |
//! |---|---|
//! | `labels` | the pod colour (hash into the theme palette) and the short-name gutter |
//! | `banner` | the notices: pod added / ended, "N more pods not streamed", "no pods match" |
//! | `sources` | the toolbar's Sources menu: switch pods and containers off and on |
//!
//! The view holds an [`AggregateSession`](oxikube_app::logs::AggregateSession): its
//! [`LogSession`](oxikube_app::logs::LogSession) is the merged buffer, read exactly like a pod's
//! (one delta pump, virtualised rows, coalesced notify); its
//! [`AggregateView`](oxikube_app::logs::AggregateView) says which streams exist, what changed in
//! the pod set and which sources are switched off. A second pump task wakes on its changes and
//! updates the gutters, the banner and, when sources were switched, the rows shown.

mod banner;
mod labels;
mod sources;

use std::time::Duration;

use futures::StreamExt as _;
use gpui::{
    App, Context, Div, FontWeight, Hsla, ParentElement as _, SharedString, Styled as _, Task, div,
};
use oxikube_app::logs::{AggregatePorts, AggregateSpec, AggregateView, HiddenSources, SourceInfo};
use oxikube_theme::ActiveTheme;
use oxikube_theme::tokens::LOG_SOURCE_COLORS;
use oxikube_ui::ActiveTokens as _;

pub use banner::{BANNER_LINES, BANNER_SECONDS, Banner};
pub use labels::{MAX_GUTTER, Prefix, SourceLabels, colour_index, short_names};
pub use sources::SourceChoice;

use super::LogView;
use super::window::LineWindow;

/// What a view of a workload or Service holds besides the merged session.
pub struct AggregateState {
    /// What is read (the object, narrowed by the options' selector and container).
    pub(crate) spec: AggregateSpec,
    /// The streams, pod events and hidden sources of the open session.
    pub(crate) session: Option<AggregateView>,
    /// The sources as of the last change, and their gutters.
    pub(crate) sources: Vec<SourceInfo>,
    pub(crate) labels: SourceLabels,
    /// What is switched off, and the counter it changed at.
    pub(crate) hidden: HiddenSources,
    pub(crate) hidden_version: u64,
    /// The last pod event shown, and the banner's content.
    pub(crate) last_event: Option<u64>,
    pub(crate) banner: Banner,
    /// Pods the stream cap left out.
    pub(crate) skipped_pods: usize,
    /// Pods the selector matches, whether or not a container of them can be read yet.
    pub(crate) matched_pods: usize,
    /// Clears the banner when its time is up (replaced by the next event, never by itself).
    pub(crate) banner_timer: Option<Task<()>>,
    /// Wakes on the aggregate's changes; replaced with the session.
    pub(crate) changes: Option<Task<()>>,
}

impl AggregateState {
    pub(crate) fn new(spec: AggregateSpec) -> Self {
        Self {
            spec,
            session: None,
            sources: Vec::new(),
            labels: SourceLabels::default(),
            hidden: HiddenSources::default(),
            hidden_version: 0,
            last_event: None,
            banner: Banner::default(),
            skipped_pods: 0,
            matched_pods: 0,
            banner_timer: None,
            changes: None,
        }
    }

    /// Forgets the open session's bookkeeping (a new session starts from nothing). What the user
    /// switched off stays, by pod and container name: [`LogView::open_aggregate_stream`] hands it
    /// to the new session.
    pub(crate) fn restart(&mut self) {
        self.session = None;
        self.hidden_version = 0;
        self.sources.clear();
        self.labels = SourceLabels::default();
        self.last_event = None;
        self.banner.clear();
        self.skipped_pods = 0;
        self.matched_pods = 0;
        self.banner_timer = None;
        self.changes = None;
    }
}

/// The palette the pods' colours come from: the active theme's `oxikube.log_sources`, else the
/// token colours before a theme is loaded.
pub(crate) fn palette(cx: &App) -> [Hsla; LOG_SOURCE_COLORS] {
    match cx.try_global::<ActiveTheme>() {
        Some(theme) => theme.0.oxikube.log_sources,
        None => {
            let c = cx.colors();
            let basic = [c.accent, c.success, c.warning, c.info];
            std::array::from_fn(|i| basic[i % basic.len()])
        }
    }
}

/// The gutter of a multi-pod line: the pod's short name in its palette colour (`None` for a pod's
/// view, whose lines have no gutter).
pub(crate) fn gutter(prefix: Option<(SharedString, usize)>, cx: &App) -> Option<Div> {
    let (text, slot) = prefix?;
    let palette = palette(cx);
    Some(
        div()
            .flex_none()
            .whitespace_nowrap()
            .text_color(palette[slot % palette.len()])
            .font_weight(FontWeight::MEDIUM)
            .child(text),
    )
}

impl LogView {
    /// Whether the view shows a workload or Service (several pods) rather than one pod.
    pub fn is_aggregate(&self) -> bool {
        self.aggregate.is_some()
    }

    /// What the view reads, by name: `deployment/web` for a multi-pod view, else the pod's name.
    pub(crate) fn subject(&self) -> String {
        match &self.aggregate {
            Some(state) => state.spec.label(),
            None => self.target.name.to_string(),
        }
    }

    /// The streams of a multi-pod view as of the last change (empty for a pod's view).
    pub fn sources(&self) -> &[SourceInfo] {
        self.aggregate.as_ref().map_or(&[], |a| &a.sources)
    }

    /// The pod events the banner shows now, oldest first (`pod web-7d9 added`).
    pub fn banner_lines(&self) -> Vec<String> {
        self.aggregate
            .as_ref()
            .map(|a| a.banner.lines())
            .unwrap_or_default()
    }

    /// Pods the `logs.max_streams` cap left out (0 for a pod's view).
    pub fn skipped_pods(&self) -> usize {
        self.aggregate.as_ref().map_or(0, |a| a.skipped_pods)
    }

    /// Whether the lines of `container` of `pod` are switched off.
    pub fn is_source_hidden(&self, pod: &str, container: &str) -> bool {
        self.aggregate
            .as_ref()
            .is_some_and(|a| a.hidden.is_hidden(pod, container))
    }

    /// The gutter of the line of `container` of `pod` (a multi-pod view).
    pub fn prefix_of(&self, pod: &str, container: &str) -> Option<&Prefix> {
        self.aggregate.as_ref()?.labels.prefix(pod, container)
    }

    /// The notice for a following view none of whose pods exist (yet).
    pub(crate) fn no_pods_notice(&self) -> Option<String> {
        let state = self.aggregate.as_ref()?;
        let selector = state.session.as_ref()?.selector()?;
        (self.is_streaming() && state.matched_pods == 0)
            .then(|| format!("No pods match {selector}"))
    }

    /// The notice for a following view whose pods exist but have no container to read yet (still
    /// creating or pending), so "no pods" is not said of them.
    pub(crate) fn waiting_pods_notice(&self) -> Option<String> {
        let state = self.aggregate.as_ref()?;
        let waiting = state.matched_pods > 0 && state.sources.is_empty() && state.skipped_pods == 0;
        (self.is_streaming() && waiting).then(|| {
            let n = state.matched_pods;
            format!(
                "Waiting for {n} {} to start",
                if n == 1 { "pod" } else { "pods" }
            )
        })
    }

    fn is_streaming(&self) -> bool {
        matches!(self.window.state(), oxikube_app::logs::LogState::Streaming)
    }

    /// Opens the merged session for the current options, dropping the previous one first.
    pub(crate) fn open_aggregate_stream(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.aggregate.as_mut() else {
            return;
        };
        state.restart();
        let session = self.deps.sessions.get(&self.target.cluster);
        let ports = session.as_ref().and_then(|s| {
            Some(AggregatePorts {
                logs: s.logs()?,
                resources: s.resources()?,
            })
        });
        let Some(ports) = ports else {
            self.reset_rows(LineWindow::with_state(super::stream::not_connected()));
            oxikube_runtime::notify_coalesced(cx);
            return;
        };
        let mut spec = state.spec.clone();
        spec.extra_selector = self.options.selector.clone();
        spec.container = self.options.container.clone();
        let (session, view) = self
            .deps
            .service
            .open_aggregate_in(
                &self.target.cluster,
                ports,
                spec,
                self.options.log_options(),
            )
            .into_parts();
        if !state.hidden.is_empty() {
            view.set_hidden(state.hidden.clone());
        }
        let mut changes = view.changes();
        state.session = Some(view);
        state.changes = Some(cx.spawn(async move |this, cx| {
            while changes.next().await.is_some() {
                if this
                    .update(cx, |view, cx| view.aggregate_changed(cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
        self.start_session(session, cx);
        // Whatever the session recorded before the pump started.
        self.aggregate_changed(cx);
    }

    /// Reads what the session changed: the sources and their gutters, the pod events, the pods
    /// the cap left out and the sources switched off, which decides the rows shown.
    pub(crate) fn aggregate_changed(&mut self, cx: &mut Context<Self>) {
        let Some(state) = self.aggregate.as_mut() else {
            return;
        };
        let Some(view) = state.session.clone() else {
            return;
        };
        let sources = view.sources();
        if sources != state.sources {
            state.labels = SourceLabels::new(&sources, LOG_SOURCE_COLORS);
            state.sources = sources;
        }
        state.skipped_pods = view.skipped_pods();
        state.matched_pods = view.matched_pods();
        let events = view.events_after(state.last_event);
        if let Some(last) = events.last() {
            state.last_event = Some(last.seq);
            for event in events {
                state.banner.push(event);
            }
            state.banner_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(BANNER_SECONDS))
                    .await;
                this.update(cx, |view, cx| {
                    if let Some(state) = view.aggregate.as_mut() {
                        state.banner.clear();
                    }
                    cx.notify();
                })
                .ok();
            }));
        }
        let (hidden, version) = view.hidden();
        if version != state.hidden_version {
            state.hidden_version = version;
            state.hidden = hidden;
            self.refilter(cx);
        }
        // Rows of a view that has not been drawn since may be gutterless: draw again.
        oxikube_runtime::notify_coalesced(cx);
    }

    /// Switches `container` of `pod` (the whole pod when `None`) off, or on again.
    pub fn toggle_source(&mut self, pod: &str, container: Option<&str>) {
        if let Some(view) = self.aggregate.as_ref().and_then(|a| a.session.as_ref()) {
            // The change comes back through the aggregate's change stream, like every other.
            view.toggle_source(pod, container);
        }
    }

    /// Hides the banner.
    pub(crate) fn dismiss_banner(&mut self, cx: &mut Context<Self>) {
        if let Some(state) = self.aggregate.as_mut() {
            state.banner.clear();
            state.banner_timer = None;
        }
        cx.notify();
    }
}

impl std::fmt::Debug for AggregateState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AggregateState")
            .field("spec", &self.spec)
            .field("sources", &self.sources.len())
            .finish_non_exhaustive()
    }
}
