//! Tests of the metrics page: the model's wording, the page over a scripted backend in a
//! workspace window, its buttons, and `cluster::ShowMetricsSettings` opening it in a cluster's tab.

mod buttons;
mod model;
mod page;
mod views;

use std::rc::Rc;

use gpui::{AppContext as _, Entity, Modifiers, Point, TestAppContext, VisualTestContext};
use oxikube_app::ProviderStatus;
use oxikube_workspace::{OpenOptions, test_support::open_workspace};

use super::test_support::{RecordingDispatcher, ScriptedBackend, cluster};
use super::{MetricsDeps, MetricsSettingsView};

/// The workspace window with the page open in it.
pub(super) struct Harness {
    pub vcx: VisualTestContext,
    pub view: Entity<MetricsSettingsView>,
    pub backend: ScriptedBackend,
    pub dispatcher: RecordingDispatcher,
}

impl Harness {
    /// Opens the page over a backend serving `status`.
    pub fn open(cx: &mut TestAppContext, status: ProviderStatus) -> Self {
        let (workspace, mut vcx) = open_workspace(cx);
        vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
        let backend = ScriptedBackend::new(status);
        let dispatcher = RecordingDispatcher::new();
        let deps = MetricsDeps {
            backend: Rc::new(backend.clone()),
            dispatcher: Rc::new(dispatcher.clone()),
        };
        let mut view = None;
        vcx.update(|window, cx| {
            let entity = cx.new(|cx| MetricsSettingsView::new(cluster(), deps, cx));
            view = Some(entity.clone());
            workspace.update(cx, |ws, cx| {
                ws.open_item_with(Box::new(entity), OpenOptions::default(), window, cx);
            });
        });
        vcx.run_until_parked();
        Self {
            vcx,
            view: view.expect("the view was built"),
            backend,
            dispatcher,
        }
    }

    pub fn read<R>(&mut self, f: impl FnOnce(&MetricsSettingsView) -> R) -> R {
        let view = self.view.clone();
        self.vcx.read(|cx| f(view.read(cx)))
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

    /// Changes the status the backend serves and lets the page follow.
    pub fn set(&mut self, status: ProviderStatus) {
        self.backend.set(status);
        self.vcx.run_until_parked();
    }
}
