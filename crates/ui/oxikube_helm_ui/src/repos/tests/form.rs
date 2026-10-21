use oxikube_app::{RepoRow, RepoSnapshot, RepoStatus};
use oxikube_domain::command::Command;

use super::entry;
use crate::repos::AddForm;

fn snapshot(existing: &[&str]) -> RepoSnapshot {
    RepoSnapshot {
        cli_available: true,
        loaded: true,
        rows: existing
            .iter()
            .map(|n| RepoRow {
                entry: entry(n),
                status: RepoStatus::Unknown,
            })
            .collect(),
        ..RepoSnapshot::default()
    }
}

fn form(name: &str, url: &str) -> AddForm {
    AddForm {
        name: name.into(),
        url: url.into(),
        ..AddForm::default()
    }
}

#[test]
fn an_untouched_form_shows_no_errors_and_cannot_be_sent() {
    let form = AddForm::default();
    assert!(form.errors(&snapshot(&[])).is_empty());
    assert!(!form.can_submit(&snapshot(&[])));
    assert!(form.is_blank());
}

#[test]
fn a_good_name_and_url_can_be_sent() {
    let form = form("bitnami", "https://charts.bitnami.com/bitnami");
    assert!(form.can_submit(&snapshot(&[])));
}

#[test]
fn bad_names_urls_and_duplicates_are_explained() {
    let snap = snapshot(&["bitnami"]);
    let errors = form("bad name", "https://x.example").errors(&snap);
    assert!(errors.name.unwrap().contains("letters, digits"));
    let errors = form("ok", "ftp://x").errors(&snap);
    assert!(errors.url.unwrap().contains("https://"));
    let errors = form("bitnami", "https://x.example").errors(&snap);
    assert!(errors.name.unwrap().contains("already exists"));
    assert!(!form("bitnami", "https://x.example").can_submit(&snap));
    let errors = form("ok", "https://bob:pw@x.example/index").errors(&snap);
    assert!(errors.url.unwrap().contains("credentials"));
    // Only the name typed: the url is missing but not yet an error.
    let errors = form("ok", "").errors(&snap);
    assert!(errors.url.is_none() && !form("ok", "").can_submit(&snap));
}

#[test]
fn a_password_needs_a_user_name() {
    let mut f = form("p", "https://p.example");
    f.password = "secret".into();
    assert!(f.errors(&snapshot(&[])).credentials.is_some());
    assert!(!f.can_submit(&snapshot(&[])));
    f.username = "bob".into();
    assert!(f.can_submit(&snapshot(&[])));
}

#[test]
fn warnings_cover_skipped_tls_and_credentials_over_plain_http() {
    let mut f = form("p", "http://p.example");
    assert_eq!(f.warnings(), Default::default());
    f.skip_tls = true;
    assert!(f.warnings().tls.is_some());
    f.username = "bob".into();
    assert!(f.warnings().plain_http.is_some());
}

#[test]
fn the_command_carries_no_password() {
    let mut f = form(" bitnami ", " https://b.example ");
    f.username = "bob".into();
    f.password = "hunter22-secret".into();
    f.skip_tls = true;
    f.ca_file = "/etc/ca.pem".into();
    let command = f.command();
    assert_eq!(
        command,
        Command::HelmRepoAdd {
            name: "bitnami".into(),
            url: "https://b.example".into(),
            username: Some("bob".into()),
            insecure_skip_tls_verify: true,
            ca_file: Some("/etc/ca.pem".into()),
        }
    );
    assert!(
        !format!(
            "{command:?}{}",
            serde_json::to_string(&command).unwrap_or_default()
        )
        .contains("hunter22")
    );
    assert_eq!(f.password().unwrap().expose(), "hunter22-secret");
}
