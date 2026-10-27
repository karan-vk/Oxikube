//! [`ServiceAccountPanel`]: the view and the work behind it.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, Subscription, Task, WeakEntity};
use oxikube_app::access::{PolicyFeed, PolicyHub};
use oxikube_app::store::ResourceStore;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ResourceRef};
use oxikube_resources_ui::detail::DetailView;
use oxikube_workspace::CommandDispatcher;

use super::kubeconfig::KubeconfigState;
use super::model::{BoundRoles, ServiceAccountFacts};

/// How often the panel looks at the index while the Role and binding feeds are being listed.
const WARMING_POLL: Duration = Duration::from_millis(200);

/// How often it looks once they are known.
const STEADY_POLL: Duration = Duration::from_secs(1);

/// Where the panel's data comes from.
enum Source {
    /// A detail view: the object, the store and the command bus are the view's.
    Detail(WeakEntity<DetailView>),
    /// Data in hand (tests, screenshots): the account and the bus its commands go to.
    Fixed {
        target: ResourceRef,
        dispatcher: Rc<dyn CommandDispatcher>,
    },
    /// An account whose store is handed over by a cell (tests): the same polling and recompute as
    /// [`Source::Detail`], without a detail view. The cell is empty while "not connected" and
    /// holds the new store after a reconnect.
    Following {
        target: ResourceRef,
        store: Rc<RefCell<Option<ResourceStore>>>,
        dispatcher: Rc<dyn CommandDispatcher>,
    },
}

/// The ServiceAccount panel. See the [module docs](super).
///
/// Live, it holds a lease on the cluster's shared [`PolicyFeed`] for as long as it exists: the
/// four RBAC subscriptions start when the detail opens and go with it. A foreground task looks at
/// the feed's version (a number) and, when it moved, recomputes the bound roles on the
/// background executor; the panel redraws only when the answer differs, so unrelated policy churn
/// costs the UI nothing.
pub struct ServiceAccountPanel {
    source: Source,
    pub(super) facts: Option<ServiceAccountFacts>,
    pub(super) bound: BoundRoles,
    feed: Option<Arc<PolicyFeed>>,
    seen: u64,
    poll: Option<Task<()>>,
    compute: Option<Task<()>>,
    /// The kubeconfig section's choices and result (E17-S09).
    pub(super) kubeconfig: KubeconfigState,
    /// A save of the kubeconfig in progress; dropping the panel abandons it.
    pub(super) kubeconfig_save: Option<Task<()>>,
    _detail: Option<Subscription>,
}

impl ServiceAccountPanel {
    /// A panel over `detail`, redrawn when it changes.
    pub fn new(detail: WeakEntity<DetailView>, cx: &mut Context<Self>) -> Self {
        let subscription = detail.upgrade().map(|entity| {
            cx.observe(&entity, |panel, _, cx| {
                panel.read_facts(cx);
                cx.notify();
            })
        });
        let mut panel = Self::bare(Source::Detail(detail));
        panel._detail = subscription;
        // The detail view builds its panel while it renders, so it is being updated and cannot be
        // read now: take the facts as soon as the render is over.
        let this = cx.entity().downgrade();
        cx.defer(move |cx| {
            this.update(cx, |panel, cx| {
                panel.read_facts(cx);
                cx.notify();
            })
            .ok();
        });
        panel.start_polling(cx);
        panel
    }

    /// A panel that follows the RBAC feed of whatever store `store` holds, as a detail view's
    /// panel does, over `facts` and without a detail view. For tests of the live path.
    pub fn following(
        target: ResourceRef,
        facts: ServiceAccountFacts,
        store: Rc<RefCell<Option<ResourceStore>>>,
        dispatcher: Rc<dyn CommandDispatcher>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut panel = Self::bare(Source::Following {
            target,
            store,
            dispatcher,
        });
        panel.facts = Some(facts);
        panel.start_polling(cx);
        panel
    }

    /// Looks at the feed now and then, until the panel is gone.
    fn start_polling(&mut self, cx: &mut Context<Self>) {
        self.poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let wait = this.update(cx, |panel, cx| panel.look(cx));
                match wait {
                    Ok(wait) => cx.background_executor().timer(wait).await,
                    Err(_) => break,
                }
            }
        }));
    }

    /// A panel over data already in hand: it follows nothing, and its links and buttons send their
    /// commands to `dispatcher`. For tests and screenshots.
    pub fn fixed(
        target: ResourceRef,
        facts: ServiceAccountFacts,
        bound: BoundRoles,
        dispatcher: Rc<dyn CommandDispatcher>,
    ) -> Self {
        let mut panel = Self::bare(Source::Fixed { target, dispatcher });
        panel.facts = Some(facts);
        panel.bound = bound;
        panel
    }

    fn bare(source: Source) -> Self {
        Self {
            source,
            facts: None,
            bound: BoundRoles::Loading,
            feed: None,
            seen: u64::MAX,
            poll: None,
            compute: None,
            kubeconfig: KubeconfigState::default(),
            kubeconfig_save: None,
            _detail: None,
        }
    }

    /// The cluster the account is in.
    pub(super) fn cluster(&self, cx: &Context<Self>) -> Option<ClusterId> {
        match &self.source {
            Source::Detail(detail) => detail
                .upgrade()
                .map(|detail| detail.read(cx).target().cluster.clone()),
            Source::Fixed { target, .. } | Source::Following { target, .. } => {
                Some(target.cluster.clone())
            }
        }
    }

    /// Opens the detail of `target` (`resource::Open`).
    pub(super) fn open(&self, target: ResourceRef, cx: &mut Context<Self>) {
        self.send(Command::ResourceOpen { target }, cx);
    }

    /// Sends `command` through the command bus.
    pub(super) fn send(&self, command: Command, cx: &mut Context<Self>) {
        match &self.source {
            Source::Detail(detail) => {
                detail
                    .update(cx, |detail, cx| detail.dispatch_command(command, cx))
                    .ok();
            }
            Source::Fixed { dispatcher, .. } | Source::Following { dispatcher, .. } => {
                dispatcher.dispatch(command, cx);
            }
        }
    }

    /// The account the detail shows.
    pub(super) fn target(&self, cx: &Context<Self>) -> Option<ResourceRef> {
        match &self.source {
            Source::Detail(detail) => detail
                .upgrade()
                .map(|detail| detail.read(cx).target().clone()),
            Source::Fixed { target, .. } | Source::Following { target, .. } => Some(target.clone()),
        }
    }

    /// Takes the account's fields from the object the detail holds.
    fn read_facts(&mut self, cx: &Context<Self>) {
        let Source::Detail(detail) = &self.source else {
            return;
        };
        let Some(detail) = detail.upgrade() else {
            return;
        };
        let detail = detail.read(cx);
        self.facts = detail.resource().map(ServiceAccountFacts::from_resource);
    }

    /// One look at the feed: takes the lease on (or moves it to) the session's current store, and
    /// recomputes when the index moved. How long to wait before the next look.
    fn look(&mut self, cx: &mut Context<Self>) -> Duration {
        // A kubeconfig being generated is looked for quickly; otherwise the same cadence as below.
        let requesting = self.refresh_kubeconfig(cx);
        let (target, store) = match &self.source {
            Source::Detail(detail) => {
                let Some(detail) = detail.upgrade() else {
                    return STEADY_POLL;
                };
                let detail = detail.read(cx);
                (detail.target().clone(), detail.columns_context().store)
            }
            Source::Following { target, store, .. } => (target.clone(), store.borrow().clone()),
            Source::Fixed { .. } => return STEADY_POLL,
        };
        let Some(store) = store else {
            // Not connected (yet): the next look takes the store the session brings.
            return WARMING_POLL;
        };
        if !self.feed.as_ref().is_some_and(|feed| feed.reads(&store)) {
            self.feed = Some(PolicyHub::shared().acquire(&store));
            self.seen = u64::MAX;
        }
        let Some(feed) = self.feed.clone() else {
            return WARMING_POLL;
        };
        let version = feed.version();
        if version != self.seen {
            self.seen = version;
            self.recompute(feed, target, cx);
        }
        if requesting || matches!(self.bound, BoundRoles::Loading) {
            WARMING_POLL
        } else {
            STEADY_POLL
        }
    }

    /// Reads the bound roles on the background executor; a newer read replaces (cancels) an older.
    fn recompute(&mut self, feed: Arc<PolicyFeed>, target: ResourceRef, cx: &mut Context<Self>) {
        let subject = oxikube_domain::rbac::Subject::service_account(
            target.namespace().unwrap_or_default(),
            &target.name,
        );
        self.compute = Some(cx.spawn(async move |this, cx| {
            let roles = cx
                .background_executor()
                .spawn(async move { BoundRoles::read(&feed, &subject) })
                .await;
            this.update(cx, |panel, cx| {
                if panel.bound != roles {
                    panel.bound = roles;
                    cx.notify();
                }
            })
            .ok();
        }));
    }
}
