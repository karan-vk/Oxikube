//! The kubeconfig text: one cluster, one user (the ServiceAccount's token) and one context.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oxikube_domain::secret::SecretText;
use serde_json::{Map, Value, json};

/// What the preview shows in the token's place.
pub const MASK: &str = "************";

/// Whether the text carries the token or a mask in its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tokens {
    /// The real token: what is copied or saved.
    Revealed,
    /// [`MASK`]: what the dialog shows by default.
    Masked,
}

/// Everything a ServiceAccount's kubeconfig is made of. The token is a [`SecretText`], so a
/// `{:?}` of the whole spec prints `***` for it.
#[derive(Debug, Clone)]
pub struct KubeconfigSpec {
    /// The kubeconfig `cluster` entry's name (the source cluster's name).
    pub cluster_name: String,
    /// The API server URL.
    pub server: String,
    /// The certificate authority as PEM text; embedded base64 as `certificate-authority-data`.
    pub certificate_authority_pem: Option<String>,
    /// Skip TLS verification, as the source kubeconfig does.
    pub insecure_skip_tls_verify: bool,
    /// The name to verify the server certificate against.
    pub tls_server_name: Option<String>,
    /// The ServiceAccount's name: the `user` entry's name, and the front of the context's name.
    pub service_account: String,
    /// The ServiceAccount's namespace: the context's default namespace.
    pub namespace: String,
    /// The bearer token.
    pub token: SecretText,
    /// A line for the top of the file as a `#` comment: when the token expires, or why a
    /// long-lived one is a risk. Not a secret.
    pub notes: Vec<String>,
}

impl KubeconfigSpec {
    /// The context's name: `<service account>@<cluster>`.
    pub fn context_name(&self) -> String {
        format!("{}@{}", self.service_account, self.cluster_name)
    }

    /// The kubeconfig as YAML, `# ` notes first.
    ///
    /// # Errors
    ///
    /// A message when the YAML writer fails (it does not for these shapes).
    pub fn render(&self, tokens: Tokens) -> Result<String, String> {
        let mut cluster = Map::new();
        cluster.insert("server".into(), json!(self.server));
        if let Some(pem) = &self.certificate_authority_pem {
            cluster.insert(
                "certificate-authority-data".into(),
                json!(STANDARD.encode(pem.as_bytes())),
            );
        }
        if self.insecure_skip_tls_verify {
            cluster.insert("insecure-skip-tls-verify".into(), json!(true));
        }
        if let Some(name) = &self.tls_server_name {
            cluster.insert("tls-server-name".into(), json!(name));
        }
        let token = match tokens {
            Tokens::Revealed => self.token.expose(),
            Tokens::Masked => MASK,
        };
        let context = self.context_name();
        let document = json!({
            "apiVersion": "v1",
            "kind": "Config",
            "clusters": [{"name": self.cluster_name, "cluster": Value::Object(cluster)}],
            "users": [{"name": self.service_account, "user": {"token": token}}],
            "contexts": [{
                "name": context,
                "context": {
                    "cluster": self.cluster_name,
                    "user": self.service_account,
                    "namespace": self.namespace,
                },
            }],
            "current-context": context,
        });
        // One line per value, as kubectl writes them: the token and the CA are long and have no
        // spaces to fold at.
        let options = serde_saphyr::ser_options! { prefer_block_scalars: false };
        let yaml = serde_saphyr::to_string_with_options(&document, options)
            .map_err(|_| "the kubeconfig cannot be written as YAML".to_owned())?;
        let mut out = String::new();
        for note in &self.notes {
            for line in note.lines() {
                out.push_str("# ");
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&yaml);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        Ok(out)
    }
}
