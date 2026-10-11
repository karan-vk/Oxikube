//! [`ServicePanel`]: the Service's own panel above the generic Overview.
//!
//! The panel follows the detail it sits in: when the detail has a new version of the Service, it
//! rebuilds its [`ServiceView`] and, when the Service's selector changed, re-subscribes its two
//! feeds. Everything it draws is computed here, off the render path; a delta from either feed
//! rebuilds the view once and redraws through [`notify_coalesced`].

use std::task::Poll;

use gpui::{AnyView, App, AppContext as _, Context, Task, WeakEntity, Window};
use oxikube_app::ServiceBackends;
use oxikube_domain::command::Command;
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::view::ServiceSummary;
use oxikube_runtime::notify_coalesced;

use super::feeds::Feeds;
use super::model::{FeedStatus, Inputs, ServiceView};
use crate::detail::DetailView;

/// The panel of one Service's detail. See the [module docs](self).
pub struct ServicePanel {
    pub(super) detail: WeakEntity<DetailView>,
    /// The Service shown; known after the first [`sync`](Self::sync) (the panel is built while
    /// the detail draws, so it cannot read it yet).
    pub(super) target: Option<ResourceRef>,
    feeds: Feeds,
    pub(super) summary: Option<ServiceSummary>,
    pub(super) view: Option<ServiceView>,
    poll: Option<Task<()>>,
    _follow: Option<gpui::Subscription>,
    /// How many times the view was rebuilt (tests check it does not churn).
    #[cfg(test)]
    pub(super) rebuilds: usize,
}

/// The factory the Service's spec registers: builds the panel for one detail view.
pub(super) fn build(detail: WeakEntity<DetailView>, _: &mut Window, cx: &mut App) -> AnyView {
    cx.new(|cx| ServicePanel::new(detail, cx)).into()
}

impl ServicePanel {
    fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let follow = detail
            .upgrade()
            .map(|view| cx.observe(&view, |this, _, cx| this.sync(cx)));
        let this = Self {
            detail,
            target: None,
            feeds: Feeds::default(),
            summary: None,
            view: None,
            poll: None,
            _follow: follow,
            #[cfg(test)]
            rebuilds: 0,
        };
        // The detail is being drawn (it builds its panel): read it once it is done.
        let weak = cx.weak_entity();
        cx.defer(move |cx| {
            weak.update(cx, |this, cx| this.sync(cx)).ok();
        });
        this
    }

    /// Reads the Service from the detail, follows its feeds and rebuilds the view.
    fn sync(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (target, summary, store) = {
            let detail = detail.read(cx);
            (
                detail.target().clone(),
                detail
                    .resource()
                    .and_then(|r| ServiceSummary::from_resource(r).ok()),
                detail.followed_store(),
            )
        };
        self.target = Some(target);
        let Some(summary) = summary else {
            return;
        };
        if let Some(store) = store {
            let backends = ServiceBackends::of(&summary);
            let key = (
                summary.namespace.clone(),
                summary.name.clone(),
                summary.selector_text(),
            );
            if self.feeds.follow(store, key, &backends) {
                self.poll = Some(Self::poll(cx));
            }
        }
        self.summary = Some(summary);
        self.rebuild(cx);
    }

    /// Rebuilds the view from the Service and what the feeds hold; redraws when it differs.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let (Some(summary), Some(target), Some(detail)) =
            (&self.summary, &self.target, self.detail.upgrade())
        else {
            return;
        };
        let (columns, now) = {
            let detail = detail.read(cx);
            (detail.deps().columns.clone(), detail.clock())
        };
        let pods = self
            .feeds
            .selects_pods
            .then_some(self.feeds.pods.as_slice());
        let none_loading = FeedStatus::Ready;
        let view = ServiceView::build(&Inputs {
            service: summary,
            target,
            slices: &self.feeds.slices,
            slices_status: &self.feeds.slices_status,
            pods,
            pods_status: if self.feeds.selects_pods {
                &self.feeds.pods_status
            } else {
                &none_loading
            },
            columns: columns.as_ref(),
            now,
        });
        #[cfg(test)]
        {
            self.rebuilds += 1;
        }
        if self.view.as_ref() != Some(&view) {
            self.view = Some(view);
            cx.notify();
        }
    }

    /// The task that polls the feeds until the panel goes.
    fn poll(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            futures::future::poll_fn(|task_cx| {
                let polled = this.update(cx, |panel, cx| {
                    let (changed, live) = panel.feeds.drain(task_cx);
                    if changed {
                        panel.rebuild(cx);
                        notify_coalesced(cx);
                    }
                    live
                });
                match polled {
                    Ok(true) => Poll::Pending,
                    _ => Poll::Ready(()),
                }
            })
            .await;
        })
    }

    /// The Service port buttons offer "Open in browser" for: sends `service::OpenInBrowser`.
    pub fn open_in_browser(&mut self, port: u16, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let Some(target) = self.target.clone() else {
            return;
        };
        let command = Command::ServiceOpenInBrowser {
            target,
            port: Some(port),
        };
        let dispatcher = detail.read(cx).deps().dispatcher.clone();
        dispatcher.dispatch(command, cx);
    }

    /// "Show pods": sends `service::ShowPods`, which opens the Pods table filtered by the
    /// Service's selector.
    pub fn show_pods(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let Some(target) = self.target.clone() else {
            return;
        };
        let command = Command::ServiceShowPods { target };
        let dispatcher = detail.read(cx).deps().dispatcher.clone();
        dispatcher.dispatch(command, cx);
    }

    /// Opens the detail of a pod or EndpointSlice row: sends `resource::Open`.
    pub(super) fn open_object(&mut self, target: ResourceRef, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let dispatcher = detail.read(cx).deps().dispatcher.clone();
        dispatcher.dispatch(Command::ResourceOpen { target }, cx);
    }

    /// The view as last built (tests read it).
    #[cfg(test)]
    pub(super) fn view(&self) -> Option<&ServiceView> {
        self.view.as_ref()
    }
}

impl Drop for ServicePanel {
    fn drop(&mut self) {
        self.feeds.release();
    }
}
