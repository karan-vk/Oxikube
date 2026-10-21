//! The add form: its text, what is wrong with it, and the command it makes.

use oxikube_app::RepoSnapshot;
use oxikube_domain::command::Command;
use oxikube_domain::helm::{RepoPassword, validate_repo};

/// What the person typed. Plain strings; nothing here is a secret except `password`, which the
/// view clears after sending.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddForm {
    /// The alias.
    pub name: String,
    /// The index URL.
    pub url: String,
    /// Basic-auth user name.
    pub username: String,
    /// Basic-auth password or token.
    pub password: String,
    /// A CA bundle path.
    pub ca_file: String,
    /// Skip TLS verification.
    pub skip_tls: bool,
}

/// What blocks the Add button, in the person's words. A field is `None` when fine or untouched.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormErrors {
    /// About the name.
    pub name: Option<String>,
    /// About the URL.
    pub url: Option<String>,
    /// About the credentials.
    pub credentials: Option<String>,
}

impl FormErrors {
    /// Nothing blocks the add.
    pub fn is_empty(&self) -> bool {
        self.name.is_none() && self.url.is_none() && self.credentials.is_none()
    }
}

/// Things that are allowed but worth a second look.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormWarnings {
    /// The certificate is not verified.
    pub tls: Option<&'static str>,
    /// Credentials would travel unencrypted.
    pub plain_http: Option<&'static str>,
}

impl AddForm {
    /// `true` while every field is blank.
    pub fn is_blank(&self) -> bool {
        self.name.is_empty() && self.url.is_empty()
    }

    /// What blocks adding, given the repositories already configured. Blank fields are not
    /// reported until something is typed, so an untouched form shows no red.
    pub fn errors(&self, existing: &RepoSnapshot) -> FormErrors {
        let (name, url) = (self.name.trim(), self.url.trim());
        let mut errors = FormErrors::default();
        if !name.is_empty() {
            errors.name = validate_repo(name, "https://x")
                .err()
                .map(str::to_owned)
                .or_else(|| {
                    existing
                        .row(name)
                        .map(|_| format!("A repository called {name} already exists."))
                });
        }
        if !url.is_empty() {
            let authority = url.split_once("://").map_or(url, |(_, rest)| rest);
            let authority = authority.split('/').next().unwrap_or_default();
            errors.url = if authority.contains('@') {
                Some("Do not put credentials in the URL; use the fields below.".to_owned())
            } else {
                validate_repo("x", url).err().map(str::to_owned)
            };
        }
        if !self.password.is_empty() && self.username.trim().is_empty() {
            errors.credentials = Some("A password needs a user name.".to_owned());
        }
        errors
    }

    /// Whether the form can be sent.
    pub fn can_submit(&self, existing: &RepoSnapshot) -> bool {
        !self.name.trim().is_empty()
            && !self.url.trim().is_empty()
            && self.errors(existing).is_empty()
    }

    /// Cautions for this input.
    pub fn warnings(&self) -> FormWarnings {
        let url = self.url.trim();
        FormWarnings {
            tls: self.skip_tls.then_some(
                "The repository's certificate will not be checked: anyone on the network could impersonate it.",
            ),
            plain_http: (url.starts_with("http://")
                && (!self.password.is_empty() || !self.username.trim().is_empty()))
            .then_some("This URL is not encrypted: the user name and password would be sent in clear text."),
        }
    }

    /// The password to stage, if one was typed.
    pub fn password(&self) -> Option<RepoPassword> {
        (!self.password.is_empty()).then(|| RepoPassword::new(self.password.clone()))
    }

    /// The `helm::RepoAdd` command. It carries no password.
    pub fn command(&self) -> Command {
        let some = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_owned())
        };
        Command::HelmRepoAdd {
            name: self.name.trim().to_owned(),
            url: self.url.trim().to_owned(),
            username: some(&self.username),
            insecure_skip_tls_verify: self.skip_tls,
            ca_file: some(&self.ca_file),
        }
    }
}
