//! The page in a workspace window.

use gpui::TestAppContext;
use oxikube_app::RepoStatus;
use oxikube_domain::OxiError;
use oxikube_domain::command::Command;
use oxikube_testkit::HelmCall;

use super::{Harness, entry};

#[gpui::test]
fn it_lists_helms_repositories_after_reading_them(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami", "jetstack"]);
    assert!(h.is_laid_out("helm-repos"));
    assert!(h.is_laid_out("helm-repos-list"));
    assert!(h.is_laid_out("helm-repo-row:bitnami"));
    assert!(h.is_laid_out("helm-repo-status:jetstack:muted"));
    assert!(h.is_laid_out("helm-repos-update-all"));
    assert!(h.is_laid_out("helm-repos-add-toggle"));
    assert!(!h.is_laid_out("helm-repos-guidance"));
    assert!(!h.is_laid_out("helm-repos-form"), "the form starts closed");
    h.read(|view| assert_eq!(view.model().rows.len(), 2));
    assert_eq!(
        h.fake.recorded_calls().len(),
        1,
        "one list, off the UI thread"
    );
}

#[gpui::test]
fn without_helm_the_page_explains_and_offers_nothing_to_change(cx: &mut TestAppContext) {
    let mut h = Harness::open_without_helm(cx);
    assert!(h.is_laid_out("helm-repos-guidance"));
    assert!(!h.is_laid_out("helm-repos-checking"));
    assert!(!h.is_laid_out("helm-repos-list"));
    assert!(!h.is_laid_out("helm-repos-update-all"));
    assert!(!h.is_laid_out("helm-repos-add-toggle"));
    assert!(
        h.fake
            .recorded_calls()
            .iter()
            .all(|c| matches!(c, HelmCall::Repos)),
        "only the list was tried"
    );
    // Even the entry points do nothing.
    h.vcx.update(|window, cx| {
        h.view.update(cx, |view, cx| view.toggle_form(window, cx));
    });
    h.settle();
    assert!(!h.read(|v| v.form_open()));
    assert!(h.bus.sent().is_empty());
}

#[gpui::test]
fn an_empty_list_invites_adding_one(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[]);
    assert!(h.is_laid_out("helm-repos-empty"));
    assert!(h.is_laid_out("helm-repos-suggestion:0"));
}

#[gpui::test]
fn a_suggestion_fills_the_form_and_adds_nothing(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[]);
    h.click("helm-repos-suggestion:0");
    assert!(h.is_laid_out("helm-repos-form"));
    let form = h.form();
    assert_eq!(form.name, "bitnami");
    assert!(form.url.starts_with("https://charts.bitnami.com"));
    assert!(h.bus.sent().is_empty(), "nothing is added silently");
}

#[gpui::test]
fn the_suggestions_follow_the_default_repos_setting(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[]);
    h.vcx.update(|_, cx| {
        crate::tests::set_settings(
            cx,
            r#"{ "helm": { "default_repos": [{ "name": "internal", "url": "https://charts.corp.example" }] } }"#,
        );
    });
    h.settle();
    let names = h.read(|v| {
        v.model()
            .suggestions
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        names,
        ["internal"],
        "the setting, hot-reloaded; nothing hard-coded"
    );
    h.click("helm-repos-suggestion:0");
    assert_eq!(h.form().url, "https://charts.corp.example");
    h.vcx.update(|_, cx| {
        crate::tests::set_settings(cx, r#"{ "helm": { "default_repos": [] } }"#);
    });
    h.settle();
    assert!(
        !h.is_laid_out("helm-repos-suggestion:0"),
        "emptied: nothing offered"
    );
}

#[gpui::test]
fn the_add_form_validates_before_it_sends_anything(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["bitnami"]);
    h.click("helm-repos-add-toggle");
    assert!(h.is_laid_out("helm-repos-form"));
    h.click("helm-repos-submit");
    assert!(
        h.bus.sent().is_empty(),
        "the Add button is off while the form is empty"
    );
    h.type_into(|v| v.name.clone(), "bitnami");
    h.type_into(|v| v.url.clone(), "https://x.example");
    assert!(h.is_laid_out("helm-repos-name-problem"), "a duplicate name");
    h.click("helm-repos-submit");
    assert!(h.bus.sent().is_empty());
    h.type_into(|v| v.name.clone(), "mine");
    h.type_into(|v| v.url.clone(), "nonsense");
    assert!(h.is_laid_out("helm-repos-url-problem"));
    h.type_into(|v| v.url.clone(), "https://x.example");
    assert!(!h.is_laid_out("helm-repos-url-problem"));
    h.type_into(|v| v.password.clone(), "pw");
    assert!(
        h.is_laid_out("helm-repos-credentials-problem"),
        "a password needs a user"
    );
}

#[gpui::test]
fn adding_sends_the_command_stages_the_password_and_clears_the_form(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[]);
    h.fake.script().repos.push_ok(vec![entry("private")]);
    h.click("helm-repos-add-toggle");
    h.type_into(|v| v.name.clone(), "private");
    h.type_into(|v| v.url.clone(), "https://private.example.com");
    h.type_into(|v| v.username.clone(), "bob");
    h.type_into(|v| v.password.clone(), "hunter22-secret");
    assert!(h.is_laid_out("helm-repos-credentials-note"));
    h.click("helm-repos-submit");
    let sent = h.bus.sent();
    assert_eq!(sent.len(), 1);
    assert!(
        matches!(&sent[0], Command::HelmRepoAdd { name, username, .. }
        if name == "private" && username.as_deref() == Some("bob"))
    );
    assert!(
        !format!("{sent:?}").contains("hunter22"),
        "the command carries no password"
    );
    // The password reached the port once, in memory.
    let add = h
        .fake
        .recorded_calls()
        .into_iter()
        .find_map(|c| match c {
            HelmCall::RepoAdd(r) => Some(r),
            _ => None,
        })
        .expect("the add ran");
    assert_eq!(add.password.unwrap().expose(), "hunter22-secret");
    // Added: the form closed and emptied, the repository is listed as fresh.
    let form = h.form();
    assert!(form.is_blank() && form.password.is_empty() && form.username.is_empty());
    h.read(|v| {
        assert!(!v.form_open());
        assert_eq!(v.model().rows.len(), 1);
        assert_eq!(
            v.model().notice.as_deref(),
            Some("Added repository private")
        );
    });
    assert!(h.is_laid_out("helm-repo-status:private:good"));
}

#[gpui::test]
fn a_failed_add_keeps_the_form_and_says_why(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &[]);
    h.fake
        .script()
        .repo_add
        .push_err(OxiError::internal("helm failed: 401 Unauthorized"));
    h.click("helm-repos-add-toggle");
    h.type_into(|v| v.name.clone(), "private");
    h.type_into(|v| v.url.clone(), "https://private.example.com");
    h.click("helm-repos-submit");
    assert!(h.read(|v| v.form_open()), "the form stays for a fix");
    assert_eq!(h.form().name, "private");
    assert!(h.is_laid_out("helm-repos-problem"));
    let problem = h.read(|v| v.model().problem.clone().unwrap());
    assert!(problem.contains("credentials"), "{problem}");
}

#[gpui::test]
fn update_all_shows_each_repositorys_result(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["a", "b"]);
    // "Update all" lists Helm again first.
    h.fake.script().repos.push_ok(vec![entry("a"), entry("b")]);
    h.fake.script().repo_update.push_ok(());
    h.fake
        .script()
        .repo_update
        .push_err(OxiError::internal("no such host"));
    h.click("helm-repos-update-all");
    assert_eq!(
        h.bus.sent(),
        vec![Command::HelmRepoUpdate { names: vec![] }]
    );
    assert!(h.is_laid_out("helm-repo-status:a:good"));
    assert!(h.is_laid_out("helm-repo-status:b:error"));
    assert!(
        h.is_laid_out("helm-repos-problem"),
        "the summary names the failure"
    );
    let status = h.service.snapshot().row("b").unwrap().status.clone();
    assert!(matches!(status, RepoStatus::Failed(why) if why.contains("could not be reached")));
}

#[gpui::test]
fn a_single_repository_updates_alone(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["a", "b"]);
    h.click("helm-repo-update:b");
    assert_eq!(
        h.bus.sent(),
        vec![Command::HelmRepoUpdate {
            names: vec!["b".into()]
        }]
    );
    assert!(h.is_laid_out("helm-repo-status:b:good"));
    assert!(h.is_laid_out("helm-repo-status:a:muted"));
}

#[gpui::test]
fn removing_asks_once_more_then_removes(cx: &mut TestAppContext) {
    let mut h = Harness::open(cx, &["a", "b"]);
    h.fake.script().repos.push_ok(vec![entry("b")]);
    h.click("helm-repo-remove:a");
    assert!(h.bus.sent().is_empty(), "the first click only asks");
    assert!(h.read(|v| v.confirm_remove.as_deref() == Some("a")));
    h.click("helm-repo-remove:a");
    assert_eq!(
        h.bus.sent(),
        vec![Command::HelmRepoRemove { name: "a".into() }]
    );
    assert!(!h.is_laid_out("helm-repo-row:a"));
    assert!(h.is_laid_out("helm-repo-row:b"));
}
