//! The cluster-info card's data: the Kubernetes version (read once per connection), the
//! distribution guess, the node count and the API server's address with any credentials cut out.

use oxikube_app::cluster_overview::Distribution;

/// Where the server version stands.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VersionState {
    /// Not asked yet, or the answer is on its way.
    #[default]
    Loading,
    /// The API server's `gitVersion`.
    Known(String),
    /// The read failed: the reason, shown instead of a version.
    Failed(String),
}

impl VersionState {
    /// The text of the card's Version row.
    pub fn text(&self) -> String {
        match self {
            Self::Loading => "…".to_owned(),
            Self::Known(version) => version.clone(),
            Self::Failed(reason) => format!("unknown ({reason})"),
        }
    }

    /// The version, when known.
    pub fn known(&self) -> Option<&str> {
        match self {
            Self::Known(version) => Some(version),
            Self::Loading | Self::Failed(_) => None,
        }
    }
}

/// The rows of the cluster-info card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoRows {
    /// The kubeconfig context.
    pub context: String,
    /// The Kubernetes version.
    pub version: String,
    /// The distribution, or "unknown".
    pub distribution: String,
    /// How many nodes (`–` before the list).
    pub nodes: String,
    /// The API server, without credentials or query.
    pub server: String,
}

/// The card's rows.
pub fn info_rows(
    context: &str,
    version: &VersionState,
    distribution: Distribution,
    nodes: Option<usize>,
    server: Option<&str>,
) -> InfoRows {
    InfoRows {
        context: context.to_owned(),
        version: version.text(),
        distribution: distribution.label().to_owned(),
        nodes: nodes.map_or_else(|| "–".to_owned(), |n| n.to_string()),
        server: server.map_or_else(|| "–".to_owned(), api_server_text),
    }
}

/// `server` as shown: scheme, host, port and path only. A user and password (`https://u:p@h`)
/// and a query (`?token=...`) are cut out, and what is left goes through the redaction patterns,
/// so a credential in the kubeconfig's `server` field never reaches the screen.
pub fn api_server_text(server: &str) -> String {
    let server = server.trim();
    let without_query = server.split(['?', '#']).next().unwrap_or_default();
    let shown = match without_query.split_once("://") {
        Some((scheme, rest)) => {
            let (authority, path) = match rest.find('/') {
                Some(i) => rest.split_at(i),
                None => (rest, ""),
            };
            let host = authority.rsplit('@').next().unwrap_or(authority);
            format!("{scheme}://{host}{path}")
        }
        None => without_query
            .rsplit('@')
            .next()
            .unwrap_or(without_query)
            .to_owned(),
    };
    oxikube_domain::redact::redact(&shown).into_owned()
}
