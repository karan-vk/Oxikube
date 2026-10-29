//! [`ResourceDiffView`]: the tab of one comparison (E20-S09), a workspace [`Item`].
//!
//! It holds the state of the comparison it was opened for ([`DiffState`]) and its two switches
//! (status, name and namespace). A switch sends `resource::DiffWith` again with the new flags:
//! the handler reads and compares anew and the same tab is updated. Nothing here reads a cluster
//! or computes a diff: the handler does, off the UI thread.

use std::rc::Rc;

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, SharedString, UniformListScrollHandle,
};
use oxikube_app::diff::DiffRequest;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_ui::IconName;
use oxikube_workspace::{CommandDispatcher, Item, ItemEvent, TabContent};

use super::data::DiffState;
use crate::view::live::DiffLayout;

/// The tab of a comparison.
pub struct ResourceDiffView {
    pub(super) request: DiffRequest,
    pub(super) state: DiffState,
    pub(super) layout: DiffLayout,
    pub(super) summary_scroll: UniformListScrollHandle,
    pub(super) lines_scroll: UniformListScrollHandle,
    dispatcher: Rc<dyn CommandDispatcher>,
    focus: FocusHandle,
}

impl ResourceDiffView {
    /// A tab for `request`, working until the first result arrives.
    pub fn new(
        request: DiffRequest,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            request,
            state: DiffState::Working("Reading both objects…".to_owned()),
            layout: DiffLayout::default(),
            summary_scroll: UniformListScrollHandle::new(),
            lines_scroll: UniformListScrollHandle::new(),
            dispatcher,
            focus: cx.focus_handle(),
        }
    }

    /// The key two tabs of one comparison share: the objects, not the switches.
    pub fn key_of(request: &DiffRequest) -> SharedString {
        format!("resource-diff:{}|{}", request.left, request.right).into()
    }

    /// The comparison this tab is about.
    pub fn request(&self) -> &DiffRequest {
        &self.request
    }

    /// What the tab is doing.
    pub fn state(&self) -> &DiffState {
        &self.state
    }

    /// The layout of the line diff.
    pub fn layout(&self) -> DiffLayout {
        self.layout
    }

    /// Takes a new state of the comparison (and its switches, which the request carries).
    pub fn update_state(&mut self, request: DiffRequest, state: DiffState, cx: &mut Context<Self>) {
        self.request = request;
        self.state = state;
        cx.notify();
    }

    /// Scrolls the line diff so row `row` is at the top (the perf scenario's scripted scroll).
    pub fn scroll_to_row(&mut self, row: usize, cx: &mut Context<Self>) {
        self.lines_scroll
            .scroll_to_item(row, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    /// Side by side or inline.
    pub fn toggle_layout(&mut self, cx: &mut Context<Self>) {
        self.layout = match self.layout {
            DiffLayout::SideBySide => DiffLayout::Inline,
            DiffLayout::Inline => DiffLayout::SideBySide,
        };
        cx.notify();
    }

    /// Compares again with `status` switched.
    pub fn toggle_status(&mut self, cx: &mut Context<Self>) {
        let include_status = !self.request.include_status;
        self.resend(include_status, self.request.compare_identity, cx);
    }

    /// Compares again with the name and namespace switched.
    pub fn toggle_identity(&mut self, cx: &mut Context<Self>) {
        let compare_identity = !self.request.compare_identity;
        self.resend(self.request.include_status, compare_identity, cx);
    }

    /// Reads both objects again (the user's refresh).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.resend(
            self.request.include_status,
            self.request.compare_identity,
            cx,
        );
    }

    fn resend(&mut self, include_status: bool, compare_identity: bool, cx: &mut Context<Self>) {
        if matches!(self.state, DiffState::Working(_)) {
            return;
        }
        self.state = DiffState::Working("Comparing…".to_owned());
        cx.notify();
        let command = Command::ResourceDiffWith {
            target: self.request.left.clone(),
            other: Some(self.request.right.clone()),
            include_status,
            compare_identity,
        };
        self.dispatcher.dispatch(command, cx);
    }

    fn title(&self) -> SharedString {
        let ResourceRef { gvk, name, .. } = &self.request.left;
        format!("Diff {}/{}", gvk.kind.to_lowercase(), name).into()
    }
}

impl EventEmitter<ItemEvent> for ResourceDiffView {}

impl Focusable for ResourceDiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Item for ResourceDiffView {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new(self.title()).icon(IconName::FileCode)
    }

    fn item_key(&self, _: &App) -> Option<SharedString> {
        Some(Self::key_of(&self.request))
    }

    // Never saved with the layout: it holds a comparison of live objects (Secret fingerprints
    // included), and nothing about them goes to disk.
}
