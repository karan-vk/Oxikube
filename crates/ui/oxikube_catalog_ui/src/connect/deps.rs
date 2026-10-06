//! [`ConnectDeps`]: what the connect view needs from the outside.

use std::rc::Rc;

use gpui::App;
use oxikube_app::ClusterSessionManager;
use oxikube_domain::ids::ClusterId;

use crate::catalog::CommandDispatcher;

/// Opens a local terminal on a cluster's context, for a re-login (the terminal, E09).
pub type OpenTerminal = dyn Fn(&ClusterId, &mut App);

/// Opens the kubeconfig sources management (E06-S05).
pub type OpenSources = dyn Fn(&mut App);

/// What the connect views of a window run on. All of it is handed in, so a test builds the view
/// over fakes and the app over its services.
#[derive(Clone)]
pub struct ConnectDeps {
    /// The sessions whose states the views follow. Only read here: retrying and cancelling are
    /// commands.
    pub sessions: ClusterSessionManager,
    /// Where `cluster::Reconnect`, `cluster::CancelConnect` and `cluster::Connect` go (the
    /// `CommandBus`, or the catalog's service dispatcher until the bus is wired).
    pub dispatcher: Rc<dyn CommandDispatcher>,
    /// Opens a terminal for the cluster. `None` until the terminal exists: the button is then
    /// drawn disabled, with the reason.
    pub open_terminal: Option<Rc<OpenTerminal>>,
    /// Opens the kubeconfig sources. `None` hides the "Edit kubeconfig sources" link.
    pub open_sources: Option<Rc<OpenSources>>,
}

impl ConnectDeps {
    /// Dependencies with no terminal and no sources page.
    pub fn new(sessions: ClusterSessionManager, dispatcher: Rc<dyn CommandDispatcher>) -> Self {
        Self {
            sessions,
            dispatcher,
            open_terminal: None,
            open_sources: None,
        }
    }

    /// Offers "Open terminal" in the `AuthRequired` body.
    #[must_use]
    pub fn with_terminal(mut self, open: impl Fn(&ClusterId, &mut App) + 'static) -> Self {
        self.open_terminal = Some(Rc::new(open));
        self
    }

    /// Offers "Edit kubeconfig sources" in the `Error` body.
    #[must_use]
    pub fn with_sources(mut self, open: impl Fn(&mut App) + 'static) -> Self {
        self.open_sources = Some(Rc::new(open));
        self
    }
}
