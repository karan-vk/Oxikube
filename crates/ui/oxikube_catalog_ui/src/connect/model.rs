//! [`ConnectViewModel`]: what the connect view shows for a session state. Plain Rust, no GPUI,
//! so every state's content is tested without a window.

use oxikube_app::ClusterSession;
use oxikube_domain::session::ClusterSessionState;
use oxikube_ports::ExecInteractivity;

use super::policy::ExecPolicy;
use super::text::{DisplayText, scrub};

/// What the view knows about the cluster besides its state. [`ConnectViewModel::of`] redacts every
/// text in it, so a view never has to remember to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectInfo {
    /// The cluster's display name, else its context name.
    pub title: String,
    /// The kubeconfig context name.
    pub context: String,
    /// The API server URL, when the catalog entry names one.
    pub server: Option<String>,
    /// The cluster's interactive exec policy.
    pub exec: ExecInteractivity,
    /// Whether the host can open a terminal for a re-login (E09 provides it).
    pub terminal: bool,
    /// Whether the host can open the kubeconfig sources (E06-S05 provides it).
    pub sources: bool,
}

impl ConnectInfo {
    /// The same facts with every text redacted.
    fn scrubbed(&self) -> Self {
        Self {
            title: scrub(&self.title),
            context: scrub(&self.context),
            server: self.server.as_deref().map(scrub),
            ..self.clone()
        }
    }

    /// The facts of `session`, plus what the host offers.
    pub fn of(session: &ClusterSession, terminal: bool, sources: bool) -> Self {
        Self {
            title: session.title().to_owned(),
            context: session.context().as_str().to_owned(),
            server: session.server().map(str::to_owned),
            exec: session.exec_interactivity(),
            terminal,
            sources,
        }
    }
}

/// The terminal button of the `AuthRequired` body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalAction {
    /// The host can open a local terminal on the cluster's context.
    Available,
    /// There is no terminal yet (E09): the button is drawn disabled and says why.
    Unavailable,
}

/// `Connecting`: a spinner, where it is connecting to, and Cancel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectingModel {
    /// The cluster's name.
    pub title: String,
    /// The context name.
    pub context: String,
    /// The API server URL, when known.
    pub server: Option<String>,
}

/// `AuthRequired`: why, what to do, the exec policy, Open terminal and Retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRequiredModel {
    /// The cluster's name.
    pub title: String,
    /// The plugin's own message (its stderr, "token expired"): summary and full text.
    pub message: DisplayText,
    /// The cluster's exec policy.
    pub policy: ExecPolicy,
    /// How to sign in, worded for the policy.
    pub instructions: &'static str,
    /// The Open terminal button.
    pub terminal: TerminalAction,
}

/// `Error`: summary, details, Retry, and the way to the kubeconfig sources.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErrorModel {
    /// The cluster's name.
    pub title: String,
    /// The error: summary and full text.
    pub message: DisplayText,
    /// The details block: the cluster, context and server, then the full error text. What
    /// "Copy details" copies.
    pub details: String,
    /// Whether the "Edit kubeconfig sources" link is offered.
    pub sources: bool,
}

/// `Degraded`: the banner above the cluster's content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DegradedModel {
    /// The cluster's name.
    pub title: String,
}

impl DegradedModel {
    /// The banner's headline.
    pub const HEADLINE: &'static str = "Some data may be stale";
    /// What is going on.
    pub const DETAIL: &'static str = "The cluster is not answering its health checks. Oxikube keeps trying; retry to start a new connection.";
}

/// `Disconnected`: nothing is connecting (a tab closes when its session does, so this is only
/// seen for a moment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisconnectedModel {
    /// The cluster's name.
    pub title: String,
}

/// What the cluster tab shows for one session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectViewModel {
    /// `Ready`: the cluster's own content, nothing of the connect view.
    Content,
    /// `Disconnected`.
    Disconnected(DisconnectedModel),
    /// `Connecting`.
    Connecting(ConnectingModel),
    /// `AuthRequired`.
    AuthRequired(AuthRequiredModel),
    /// `Degraded`: the cluster's content with a banner above it.
    Degraded(DegradedModel),
    /// `Error`.
    Error(ErrorModel),
}

impl ConnectViewModel {
    /// The model of `state`.
    pub fn of(state: &ClusterSessionState, info: &ConnectInfo) -> Self {
        let info = &info.scrubbed();
        let policy = ExecPolicy::of(info.exec);
        match state {
            ClusterSessionState::Ready => Self::Content,
            ClusterSessionState::Disconnected => Self::Disconnected(DisconnectedModel {
                title: info.title.clone(),
            }),
            ClusterSessionState::Connecting => Self::Connecting(ConnectingModel {
                title: info.title.clone(),
                context: info.context.clone(),
                server: info.server.clone(),
            }),
            ClusterSessionState::AuthRequired { reason } => Self::AuthRequired(AuthRequiredModel {
                title: info.title.clone(),
                message: DisplayText::new(reason),
                policy,
                instructions: policy.instructions(info.terminal),
                terminal: if info.terminal {
                    TerminalAction::Available
                } else {
                    TerminalAction::Unavailable
                },
            }),
            ClusterSessionState::Degraded => Self::Degraded(DegradedModel {
                title: info.title.clone(),
            }),
            ClusterSessionState::Error { reason } => {
                let message = DisplayText::new(reason);
                Self::Error(ErrorModel {
                    details: error_details(info, &message),
                    title: info.title.clone(),
                    message,
                    sources: info.sources,
                })
            }
        }
    }
}

/// The text "Copy details" puts on the clipboard: enough for a bug report, nothing secret.
fn error_details(info: &ConnectInfo, message: &DisplayText) -> String {
    let mut details = format!("cluster: {}\ncontext: {}\n", info.title, info.context);
    if let Some(server) = &info.server {
        details.push_str(&format!("server: {server}\n"));
    }
    details.push_str(&format!(
        "exec_interactivity: {}\n\n{}",
        ExecPolicy::of(info.exec).setting_value(),
        message.full
    ));
    details
}
