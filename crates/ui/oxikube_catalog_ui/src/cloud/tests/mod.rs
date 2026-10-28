//! Tests of the Cloud section: the model and the remediation table as plain Rust, the view in a
//! workspace over a scripted backend, and the whole thing over the real service on fakes.

mod end_to_end;
mod model;
mod remediation;
mod view;

use std::rc::Rc;

use gpui::{
    AppContext as _, Entity, KeyBinding, Modifiers, Point, TestAppContext, VisualTestContext,
};
use oxikube_app::cloud::CloudDiscoveryEvent;
use oxikube_domain::cloud::{
    CloudDetection, CloudProvider, CloudToolStatus, DiscoveredCluster, SetupHint,
};
use oxikube_runtime::FRAME_INTERVAL;
use oxikube_testkit::FakeClockPort;
use oxikube_workspace::OpenOptions;
use oxikube_workspace::test_support::open_workspace;

use super::actions::{Cancel, Refresh};
use super::backend::CloudBackend;
use super::test_support::ScriptedBackend;
use super::{CloudDeps, CloudView};

/// The workspace window with the Cloud section open in it.
pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<CloudView>,
    pub clock: Arc<FakeClockPort>,
}

use std::sync::Arc;

impl Harness {
    /// Opens the section over `backend` and lets its first frame and first refresh happen.
    pub fn open(cx: &mut TestAppContext, backend: Rc<dyn CloudBackend>) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| {
            oxikube_runtime::init_deterministic(cx);
            // The defaults of `oxikube_assets` bind these in the `CatalogCloud` context.
            cx.bind_keys([
                KeyBinding::new("escape", Cancel, Some("CatalogCloud")),
                KeyBinding::new("secondary-r", Refresh, Some("CatalogCloud")),
            ]);
        });
        let clock = Arc::new(FakeClockPort::new(
            jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp"),
        ));
        let deps = CloudDeps {
            backend,
            clock: clock.clone(),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| CloudView::new(deps, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        Self {
            vcx,
            view: view.expect("the view was built"),
            clock,
        }
    }

    /// Opens the section over a scripted backend.
    pub fn scripted(cx: &mut TestAppContext) -> (Self, ScriptedBackend) {
        let backend = ScriptedBackend::new();
        (Self::open(cx, Rc::new(backend.clone())), backend)
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&CloudView) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx)))
    }

    pub fn update<R>(
        &mut self,
        f: impl FnOnce(&mut CloudView, &mut gpui::Context<CloudView>) -> R,
    ) -> R {
        let view = self.view.clone();
        self.vcx.update(|_, cx| view.update(cx, f))
    }

    /// Plays events to the view and lets it draw them.
    pub fn play(&mut self, backend: &ScriptedBackend, events: Vec<CloudDiscoveryEvent>) {
        for event in events {
            backend.emit(event);
        }
        self.vcx.run_until_parked();
        // The view redraws at frame cadence (`notify_coalesced`), not per event.
        self.vcx.executor().advance_clock(FRAME_INTERVAL * 2);
        self.vcx.run_until_parked();
    }

    pub fn bounds(&mut self, selector: &str) -> Option<gpui::Bounds<gpui::Pixels>> {
        // GPUI wants a `'static` selector: leak the few a test asks for.
        let selector: &'static str = Box::leak(selector.to_owned().into_boxed_str());
        self.vcx.debug_bounds(selector)
    }

    pub fn is_laid_out(&mut self, selector: &str) -> bool {
        self.bounds(selector).is_some()
    }

    pub fn centre(&mut self, selector: &str) -> Point<gpui::Pixels> {
        self.bounds(selector)
            .unwrap_or_else(|| panic!("{selector} was not laid out"))
            .center()
    }

    pub fn click(&mut self, selector: &str) {
        let at = self.centre(selector);
        self.vcx.simulate_click(at, Modifiers::none());
        self.vcx.run_until_parked();
    }
}

/// A detection that found the CLI, logged in.
pub(super) fn ready(provider: CloudProvider) -> CloudDetection {
    CloudDetection {
        provider,
        status: CloudToolStatus::Ready,
        binary: Some(format!("/usr/local/bin/{}", provider.cli()).into()),
        version: Some("2.15.0".into()),
        hints: Vec::new(),
    }
}

/// A detection that found the CLI with `hints` to act on.
pub(super) fn degraded(provider: CloudProvider, hints: Vec<SetupHint>) -> CloudDetection {
    CloudDetection {
        status: CloudToolStatus::NotAuthenticated,
        hints,
        ..ready(provider)
    }
}

/// The events of a run that finds `clusters` of one scope.
pub(super) fn run_events(
    provider: CloudProvider,
    detection: CloudDetection,
    scope: oxikube_domain::cloud::CloudScope,
    clusters: Vec<DiscoveredCluster>,
) -> Vec<CloudDiscoveryEvent> {
    vec![
        CloudDiscoveryEvent::Detected(detection),
        CloudDiscoveryEvent::Started {
            provider,
            scopes: vec![scope.clone()],
        },
        CloudDiscoveryEvent::ScopeDone {
            scope,
            clusters: clusters.clone(),
        },
        CloudDiscoveryEvent::Finished {
            provider,
            clusters,
            failed_scopes: 0,
        },
    ]
}
