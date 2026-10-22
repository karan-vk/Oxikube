use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use oxikube_domain::helm::{RepoAddRequest, RepoPassword};
use oxikube_domain::{ErrorKind, OxiError, OxiResult};
use oxikube_ports::{HelmCapabilities, HelmRepoPort};
use oxikube_testkit::{FakeClockPort, FakeHelmPort, HelmCall};
use parking_lot::Mutex;

use super::{entry, service};
use crate::helm_repos::{HelmReposService, RepoActivity, RepoStatus, default_suggestions};

fn fake_with(repos: &[&str]) -> Arc<FakeHelmPort> {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script()
        .repos
        .push_ok(repos.iter().map(|n| entry(n)).collect());
    fake
}

#[tokio::test]
async fn refresh_lists_helms_repositories() {
    let fake = fake_with(&["bitnami", "jetstack"]);
    let service = service(&fake);
    assert!(!service.snapshot().loaded);
    service.refresh().await.unwrap();
    let snapshot = service.snapshot();
    assert!(snapshot.loaded && snapshot.cli_available);
    let names: Vec<_> = snapshot
        .rows
        .iter()
        .map(|r| r.entry.name.as_str())
        .collect();
    assert_eq!(names, ["bitnami", "jetstack"]);
    assert!(
        snapshot
            .rows
            .iter()
            .all(|r| r.status == RepoStatus::Unknown)
    );
    assert_eq!(snapshot.activity, RepoActivity::Idle);
}

#[tokio::test]
async fn add_calls_helm_lists_again_and_cues_the_catalog() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script().repos.push_ok(vec![entry("bitnami")]);
    let service = service(&fake);
    let mut catalog = service.catalog_changes();
    let before = catalog.generation();
    service
        .add(RepoAddRequest::new(
            "bitnami",
            "https://bitnami.example.com",
        ))
        .await
        .unwrap();
    assert!(catalog.changed().await);
    assert!(catalog.generation() > before);
    let snapshot = service.snapshot();
    assert_eq!(snapshot.rows.len(), 1);
    assert!(matches!(snapshot.rows[0].status, RepoStatus::Updated(_)));
    assert_eq!(snapshot.notice.as_deref(), Some("Added repository bitnami"));
    let calls = fake.recorded_calls();
    assert!(matches!(&calls[0], HelmCall::RepoAdd(r) if r.name == "bitnami"));
    assert!(matches!(&calls[1], HelmCall::Repos));
}

#[tokio::test]
async fn a_bad_name_or_url_never_reaches_helm() {
    let fake = Arc::new(FakeHelmPort::new());
    let service = service(&fake);
    let error = service
        .add(RepoAddRequest::new("-x", "https://a.example"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Validation);
    assert!(fake.recorded_calls().is_empty());
}

#[tokio::test]
async fn a_failed_add_shows_a_readable_problem_and_changes_nothing() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script().repo_add.push_err(OxiError::internal(
        "helm failed (exit code Some(1)): looks like \"https://x.example\" is not a valid chart repository: 401 Unauthorized",
    ));
    let service = service(&fake);
    let catalog = service.catalog_changes();
    let error = service
        .add(RepoAddRequest::new("x", "https://x.example"))
        .await
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    let snapshot = service.snapshot();
    let problem = snapshot.problem.unwrap();
    assert!(problem.contains("401"), "{problem}");
    assert!(problem.contains("credentials"), "{problem}");
    assert!(snapshot.rows.is_empty());
    assert!(!catalog.has_changed(), "no cache invalidation on failure");
}

#[tokio::test]
async fn remove_lists_again() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script().repos.push_ok(vec![entry("a"), entry("b")]);
    fake.script().repos.push_ok(vec![entry("b")]);
    let service = service(&fake);
    service.refresh().await.unwrap();
    service.remove("a").await.unwrap();
    assert_eq!(service.snapshot().rows.len(), 1);
    assert!(
        fake.recorded_calls()
            .contains(&HelmCall::RepoRemove("a".into()))
    );
}

#[tokio::test]
async fn update_all_goes_repository_by_repository_and_reports_each() {
    let fake = fake_with(&["a", "b", "c"]);
    // "Every repository" is what Helm lists when the update starts.
    fake.script()
        .repos
        .push_ok(["a", "b", "c"].iter().map(|n| entry(n)).collect());
    fake.script().repo_update.push_ok(());
    fake.script()
        .repo_update
        .push_err(OxiError::internal("no such host"));
    fake.script().repo_update.push_ok(());
    let service = service(&fake);
    service.refresh().await.unwrap();
    let mut catalog = service.catalog_changes();
    let report = service.update(&[]).await.unwrap();
    assert_eq!(report.updated, ["a", "c"]);
    assert_eq!(report.failed.len(), 1);
    assert_eq!(report.failed[0].0, "b");
    assert!(report.failed[0].1.contains("could not be reached"));
    assert!(report.summary().starts_with("Updated 2, failed b ("));
    let snapshot = service.snapshot();
    assert!(matches!(
        snapshot.row("a").unwrap().status,
        RepoStatus::Updated(_)
    ));
    assert!(matches!(
        snapshot.row("b").unwrap().status,
        RepoStatus::Failed(_)
    ));
    assert!(snapshot.problem.is_some());
    let updates: Vec<_> = fake
        .recorded_calls()
        .into_iter()
        .filter_map(|c| match c {
            HelmCall::RepoUpdate(names) => Some(names),
            _ => None,
        })
        .collect();
    assert_eq!(updates, [["a"], ["b"], ["c"]]);
    assert!(
        catalog.changed().await,
        "partial success still stales the catalog"
    );
}

#[tokio::test]
async fn without_the_binary_nothing_runs_and_the_snapshot_says_so() {
    let fake = Arc::new(FakeHelmPort::with_capabilities(
        HelmCapabilities::native_only(),
    ));
    let service = service(&fake);
    assert!(!service.snapshot().cli_available);
    let error = service.refresh().await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let error = service.update(&["a".into()]).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
}

#[tokio::test]
async fn a_staged_password_goes_to_the_add_once_and_is_forgotten() {
    let fake = Arc::new(FakeHelmPort::new());
    let service = service(&fake);
    service.stage_password("private", RepoPassword::new("hunter22-secret"));
    service
        .add(RepoAddRequest::new("private", "https://p.example.com"))
        .await
        .unwrap();
    let HelmCall::RepoAdd(request) = fake.recorded_calls().remove(0) else {
        panic!("first call is the add");
    };
    assert_eq!(request.password.unwrap().expose(), "hunter22-secret");
    service
        .add(RepoAddRequest::new("private", "https://p.example.com"))
        .await
        .unwrap();
    let HelmCall::RepoAdd(second) = fake
        .recorded_calls()
        .into_iter()
        .filter(|c| matches!(c, HelmCall::RepoAdd(_)))
        .nth(1)
        .unwrap()
    else {
        unreachable!()
    };
    assert!(second.password.is_none(), "consumed by the first add");
    service.stage_password("x", RepoPassword::new("p"));
    service.discard_password("x");
}

#[test]
fn suggestions_come_from_the_setting_and_skip_what_is_configured() {
    let setting = |name: &str, url: &str| oxikube_domain::helm::RepoEntry {
        name: name.into(),
        url: url.into(),
    };
    let defaults = [
        setting("internal", "https://charts.corp.example"),
        setting("bitnami", "https://charts.bitnami.com/bitnami"),
    ];
    // Exactly the setting, in its order: nothing hard-coded is added.
    let all = default_suggestions(&defaults, &[]);
    assert_eq!(all, defaults.to_vec());
    let some = default_suggestions(&defaults, &[entry("bitnami")]);
    assert_eq!(some, vec![defaults[0].clone()]);
    let by_url = default_suggestions(
        &defaults,
        &[setting("mine", "https://charts.corp.example/")],
    );
    assert!(!by_url.iter().any(|s| s.name == "internal"));
    // An emptied setting offers nothing.
    assert!(default_suggestions(&[], &[]).is_empty());
}

#[tokio::test]
async fn a_changed_binary_reads_the_list_again_and_restarts_the_catalog() {
    let fake = fake_with(&["bitnami"]);
    let service = service(&fake);
    let catalog = service.catalog_changes();
    let before = catalog.generation();
    service.binary_changed().await.unwrap();
    assert!(service.snapshot().row("bitnami").is_some());
    assert_eq!(catalog.generation(), before + 1);
}

/// A repo port that records when each operation starts and ends, to prove they never overlap.
struct Slow {
    log: Mutex<Vec<String>>,
}

#[async_trait]
impl HelmRepoPort for Slow {
    async fn repo_add(&self, request: &RepoAddRequest) -> OxiResult<()> {
        self.log.lock().push(format!("start add {}", request.name));
        tokio::time::sleep(Duration::from_millis(30)).await;
        self.log.lock().push(format!("end add {}", request.name));
        Ok(())
    }
    async fn repo_remove(&self, name: &str) -> OxiResult<()> {
        self.log.lock().push(format!("start remove {name}"));
        tokio::time::sleep(Duration::from_millis(10)).await;
        self.log.lock().push(format!("end remove {name}"));
        Ok(())
    }
    async fn repo_update(&self, _: &[String]) -> OxiResult<()> {
        Ok(())
    }
}

#[tokio::test]
async fn our_own_repository_changes_never_overlap() {
    let helm = Arc::new(FakeHelmPort::new());
    let slow = Arc::new(Slow {
        log: Mutex::new(Vec::new()),
    });
    let service = HelmReposService::new(helm, slow.clone(), Arc::new(FakeClockPort::default()));
    let (a, b, c) = (service.clone(), service.clone(), service.clone());
    let first = tokio::spawn(async move {
        a.add(RepoAddRequest::new("one", "https://one.example"))
            .await
    });
    tokio::task::yield_now().await;
    let second = tokio::spawn(async move { b.remove("two").await });
    tokio::task::yield_now().await;
    let third = tokio::spawn(async move {
        c.add(RepoAddRequest::new("three", "https://three.example"))
            .await
    });
    for task in [first, second, third] {
        task.await.unwrap().unwrap();
    }
    let log = slow.log.lock().clone();
    assert_eq!(
        log,
        [
            "start add one",
            "end add one",
            "start remove two",
            "end remove two",
            "start add three",
            "end add three"
        ]
    );
}

#[tokio::test]
async fn update_all_lists_helm_first_so_an_unread_page_still_updates_everything() {
    let fake = fake_with(&["a", "b"]);
    let service = service(&fake);
    assert!(!service.snapshot().loaded, "nothing was read yet");
    let report = service.update(&[]).await.unwrap();
    assert_eq!(report.updated, ["a", "b"]);
}

#[tokio::test]
async fn an_unsupported_update_keeps_what_finished_and_still_stales_the_catalog() {
    let fake = fake_with(&["a", "b"]);
    fake.script().repo_update.push_ok(());
    fake.script().repo_update.push_err(OxiError::from(
        oxikube_domain::helm::HelmError::BinaryMissing,
    ));
    let service = service(&fake);
    service.refresh().await.unwrap();
    let catalog = service.catalog_changes();
    let error = service.update(&["a".into(), "b".into()]).await.unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    let snapshot = service.snapshot();
    assert!(matches!(
        snapshot.row("a").unwrap().status,
        RepoStatus::Updated(_)
    ));
    assert_eq!(snapshot.row("b").unwrap().status, RepoStatus::Unknown);
    assert!(catalog.has_changed());
}

#[tokio::test]
async fn an_add_whose_relist_fails_says_so_instead_of_claiming_success() {
    let fake = Arc::new(FakeHelmPort::new());
    fake.script()
        .repos
        .push_err(OxiError::internal("helm failed: lock"));
    let service = service(&fake);
    service
        .add(RepoAddRequest::new("x", "https://x.example"))
        .await
        .unwrap();
    let snapshot = service.snapshot();
    let problem = snapshot.problem.expect("a problem");
    assert!(
        problem.starts_with("Added x, but reading Helm's list again failed"),
        "{problem}"
    );
}

#[tokio::test]
async fn an_agents_add_never_uses_the_password_the_user_staged() {
    let fake = Arc::new(FakeHelmPort::new());
    let service = service(&fake);
    service.stage_password("p", RepoPassword::new("user-typed-secret"));
    service
        .add_with(RepoAddRequest::new("p", "https://p.example"), false)
        .await
        .unwrap();
    let HelmCall::RepoAdd(request) = fake.recorded_calls().remove(0) else {
        panic!("add first");
    };
    assert!(request.password.is_none());
}

#[tokio::test]
async fn a_staged_password_is_forgotten_after_two_minutes() {
    let fake = Arc::new(FakeHelmPort::new());
    let clock = Arc::new(FakeClockPort::default());
    let service = HelmReposService::new(fake.clone(), fake.clone(), clock.clone());
    service.stage_password("p", RepoPassword::new("secret-1"));
    clock.advance(Duration::from_secs(121));
    service
        .add(RepoAddRequest::new("p", "https://p.example"))
        .await
        .unwrap();
    let HelmCall::RepoAdd(request) = fake.recorded_calls().remove(0) else {
        panic!("add first");
    };
    assert!(request.password.is_none(), "too old to use");
}
