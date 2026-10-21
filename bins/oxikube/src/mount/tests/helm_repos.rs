//! The Helm repositories page in the running app (E16-S07): the commands on the bus with their
//! tool stubs, the sidebar's Helm > Repositories entry and `helm::ShowRepos` opening the page in
//! the connected cluster's tab, and the page's buttons running `helm::Repo*` on the bus.

use std::sync::Arc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_app::{ChartsService, HelmReposService};
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::helm::RepoEntry;
use oxikube_helm_ui::repos::HelmReposView;
use oxikube_testkit::{FakeClockPort, FakeHelmPort, HelmCall, TestPorts};

use super::App;
use crate::app_state::AppState;
use crate::mount::helm_repos::{HelmServices, ServiceOverride};

pub(super) fn fake_with(repos: &[&str]) -> Arc<FakeHelmPort> {
    let fake = Arc::new(FakeHelmPort::new());
    let entries = |names: &[&str]| -> Vec<RepoEntry> {
        names
            .iter()
            .map(|n| RepoEntry {
                name: (*n).into(),
                url: format!("https://{n}.example.com"),
            })
            .collect()
    };
    // The page lists on open, and after each change.
    for _ in 0..4 {
        fake.script().repos.push_ok(entries(repos));
    }
    fake
}

pub(super) fn start(cx: &mut TestAppContext, fake: &Arc<FakeHelmPort>) -> App {
    let repos = HelmReposService::new(
        fake.clone(),
        fake.clone(),
        Arc::new(FakeClockPort::default()),
    );
    let charts = ChartsService::new(fake.clone(), repos.catalog_changes());
    App::start_with(cx, TestPorts::seeded(), move |cx| {
        cx.set_global(ServiceOverride(HelmServices { repos, charts }))
    })
}

pub(super) fn run(app: &mut App, command: Command) -> bool {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome.is_ok()
}

fn pages(app: &mut App) -> usize {
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    app.vcx.update(|_, cx| {
        tab.read(cx)
            .workspace()
            .read(cx)
            .items_of_type::<HelmReposView>()
            .len()
    })
}

#[gpui::test]
fn the_four_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = start(cx, &fake_with(&[]));
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus");
    for (id, owner, tool) in [
        (
            CommandId::HELM_SHOW_REPOS,
            "oxikube_settings_ui::helm_repos",
            "helm.show_repos",
        ),
        (
            CommandId::HELM_REPO_ADD,
            "oxikube_app::helm_repos",
            "helm.repo_add",
        ),
        (
            CommandId::HELM_REPO_REMOVE,
            "oxikube_app::helm_repos",
            "helm.repo_remove",
        ),
        (
            CommandId::HELM_REPO_UPDATE,
            "oxikube_app::helm_repos",
            "helm.repo_update",
        ),
    ] {
        assert_eq!(bus.owner(id), Some(owner), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn the_sidebar_entry_opens_the_page_and_it_lists_helms_repositories(cx: &mut TestAppContext) {
    let mut app = start(cx, &fake_with(&["bitnami", "jetstack"]));
    app.press("enter");
    assert!(
        app.drawn("sidebar-section-helm"),
        "the Helm section is in the sidebar"
    );
    assert_eq!(pages(&mut app), 0, "nothing opens by itself");

    app.click("sidebar-entry-helm/repositories");
    assert_eq!(pages(&mut app), 1, "one page in the cluster's tab");
    assert!(app.drawn("helm-repos"));
    assert!(app.drawn("helm-repo-row:bitnami"));
    assert!(app.drawn("helm-repo-row:jetstack"));

    // Asking again (the palette's command) shows the open page.
    assert!(run(&mut app, Command::HelmShowRepos));
    assert_eq!(pages(&mut app), 1);
}

#[gpui::test]
fn the_pages_buttons_run_the_repo_commands_on_the_bus(cx: &mut TestAppContext) {
    let fake = fake_with(&["bitnami", "jetstack"]);
    let mut app = start(cx, &fake);
    app.press("enter");
    assert!(run(&mut app, Command::HelmShowRepos));
    assert!(app.drawn("helm-repos-update-all"));

    app.click("helm-repos-update-all");
    app.vcx.run_until_parked();
    let calls = fake.recorded_calls();
    assert!(
        calls.contains(&HelmCall::RepoUpdate(vec!["bitnami".into()])),
        "{calls:?}"
    );
    assert!(calls.contains(&HelmCall::RepoUpdate(vec!["jetstack".into()])));
    assert!(app.drawn("helm-repo-status:bitnami:good"));

    app.click("helm-repo-remove:bitnami");
    app.click("helm-repo-remove:bitnami");
    assert!(
        fake.recorded_calls()
            .contains(&HelmCall::RepoRemove("bitnami".into()))
    );
}

#[gpui::test]
fn an_agent_may_not_add_a_repository_without_permission(cx: &mut TestAppContext) {
    let fake = fake_with(&[]);
    let mut app = start(cx, &fake);
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    let add = Command::HelmRepoAdd {
        name: "evil".into(),
        url: "https://evil.example.com".into(),
        username: None,
        insecure_skip_tls_verify: false,
        ca_file: None,
    };
    let refused = block_on(bus.dispatch(add, DispatchContext::new(Initiator::Agent, "agent")));
    assert!(
        refused.is_err(),
        "denied until the agent epic's prompt exists"
    );
    assert!(
        !fake
            .recorded_calls()
            .iter()
            .any(|c| matches!(c, HelmCall::RepoAdd(_)))
    );
}
