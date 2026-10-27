//! The section in a workspace over a scripted backend: cards, rows, buttons, remediation, the
//! first refresh, coalescing and Escape.

use gpui::{AppContext as _, TestAppContext};
use oxikube_app::cloud::CloudDiscoveryEvent;
use oxikube_domain::OxiError;
use oxikube_domain::cloud::{CloudDetection, CloudProvider, SetupHint};
use oxikube_domain::command::{CloudRefreshMode, Command};
use oxikube_domain::ids::ContextName;
use oxikube_ports::ClockPort as _;

use super::{Harness, degraded, ready, run_events};
use crate::cloud::test_support::{ScriptedBackend, aws_scope, eks, managed};
use crate::cloud::{CloudDeps, CloudView, EMPTY_STEPS};

fn aws_run(h: &mut Harness, backend: &ScriptedBackend, names: &[&str]) {
    let scope = aws_scope("prod", "eu-west-1");
    let clusters = names.iter().map(|name| eks(&scope, name)).collect();
    let events = run_events(
        CloudProvider::Aws,
        ready(CloudProvider::Aws),
        scope,
        clusters,
    );
    h.play(backend, events);
}

#[gpui::test]
fn the_first_refresh_is_asked_for_after_the_first_frame_and_only_once(cx: &mut TestAppContext) {
    let (h, backend) = Harness::scripted(cx);
    assert_eq!(
        backend.sent(),
        [Command::CloudRefresh {
            provider: None,
            mode: CloudRefreshMode::IfStale,
        }],
        "the cache answers first; the CLIs run only when it is stale"
    );
    drop(h);
}

#[gpui::test]
fn nothing_is_asked_for_before_the_first_frame(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let backend = ScriptedBackend::new();
    let deps = CloudDeps {
        backend: std::rc::Rc::new(backend.clone()),
        clock: std::sync::Arc::new(oxikube_testkit::FakeClockPort::new(
            jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp"),
        )),
    };
    vcx.update(|window, cx| {
        let view = cx.new(|cx| CloudView::new(deps, cx));
        assert!(backend.sent().is_empty(), "construction runs nothing");
        workspace.update(cx, |ws, cx| {
            ws.open_item_with(Box::new(view), Default::default(), window, cx);
        });
        assert!(backend.sent().is_empty(), "nor does mounting it");
    });
    vcx.run_until_parked();
    assert_eq!(
        backend.sent().len(),
        1,
        "the frame triggers the first refresh"
    );
}

#[gpui::test]
fn without_a_cli_the_section_explains_instead_of_listing(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    h.play(
        &backend,
        vec![
            CloudDiscoveryEvent::Detected(CloudDetection::not_installed(CloudProvider::Aws)),
            CloudDiscoveryEvent::Detected(CloudDetection::not_installed(CloudProvider::Gcp)),
            CloudDiscoveryEvent::Detected(CloudDetection::not_installed(CloudProvider::Azure)),
        ],
    );
    assert!(h.is_laid_out("cloud-empty"));
    for ix in 0..EMPTY_STEPS.len() {
        assert!(h.is_laid_out(&format!("cloud-empty-step-{ix}")));
    }
    for provider in CloudProvider::ALL {
        assert!(!h.is_laid_out(&format!("cloud-card-{}", provider.id())));
    }
}

#[gpui::test]
fn only_detected_providers_get_a_card(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    h.play(
        &backend,
        vec![
            CloudDiscoveryEvent::Detected(CloudDetection::not_installed(CloudProvider::Aws)),
            CloudDiscoveryEvent::Detected(ready(CloudProvider::Gcp)),
        ],
    );
    assert!(h.is_laid_out("cloud-card-gcp"));
    assert!(
        !h.is_laid_out("cloud-card-aws"),
        "undetected providers are hidden"
    );
    assert!(!h.is_laid_out("cloud-card-azure"));
}

#[gpui::test]
fn clusters_show_their_location_version_and_status(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    aws_run(&mut h, &backend, &["web", "api"]);
    for ix in 0..2 {
        assert!(h.is_laid_out(&format!("cloud-row-{ix}")));
        assert!(h.is_laid_out(&format!("cloud-location-{ix}")));
        assert!(h.is_laid_out(&format!("cloud-version-{ix}")));
        assert!(h.is_laid_out(&format!("cloud-status-{ix}:Running")));
        assert!(h.is_laid_out(&format!("cloud-in-catalog-{ix}:false")));
    }
    h.read(|view| {
        let names: Vec<_> = view
            .model()
            .provider(CloudProvider::Aws)
            .shown()
            .map(|c| c.name.clone())
            .collect();
        assert_eq!(names, ["api", "web"], "sorted by key");
    });
    assert!(h.is_laid_out("cloud-card-status-aws:2 clusters"));
}

#[gpui::test]
fn progress_shows_scopes_done_of_total(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    backend.hold(true);
    let scopes: Vec<_> = ["eu-west-1", "us-east-1", "ap-south-1"]
        .iter()
        .map(|region| aws_scope("prod", region))
        .collect();
    h.play(
        &backend,
        vec![
            CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws)),
            CloudDiscoveryEvent::Started {
                provider: CloudProvider::Aws,
                scopes: scopes.clone(),
            },
            CloudDiscoveryEvent::ScopeDone {
                scope: scopes[0].clone(),
                clusters: vec![eks(&scopes[0], "web")],
            },
        ],
    );
    assert!(h.is_laid_out("cloud-card-status-aws:Listing 1 of 3 scopes"));
    assert!(h.is_laid_out("cloud-row-0"), "results show as they arrive");
}

#[gpui::test]
fn add_dispatches_the_command_and_added_clusters_are_marked(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    aws_run(&mut h, &backend, &["web"]);
    let web = eks(&aws_scope("prod", "eu-west-1"), "web");

    h.click("cloud-add-0");
    assert_eq!(
        backend.sent().last(),
        Some(&Command::CloudAdd {
            provider: CloudProvider::Aws,
            cluster: web.key(),
        })
    );
    // The backend now reports it as added: the marker shows and Add becomes Remove.
    backend.set_added(vec![managed(&web)]);
    h.update(|view, cx| view.reload_added(cx));
    h.vcx.run_until_parked();
    assert!(h.is_laid_out("cloud-in-catalog-0:true"));
    assert!(h.is_laid_out("cloud-remove-0"));
    assert!(!h.is_laid_out("cloud-add-0"));
}

#[gpui::test]
fn remove_asks_first_then_dispatches_the_command(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    let web = eks(&aws_scope("prod", "eu-west-1"), "web");
    backend.set_added(vec![managed(&web)]);
    aws_run(&mut h, &backend, &["web"]);
    h.update(|view, cx| view.reload_added(cx));
    h.vcx.run_until_parked();

    let before = backend.sent().len();
    h.click("cloud-remove-0");
    assert_eq!(backend.sent().len(), before, "the first click only asks");
    assert!(h.is_laid_out("cloud-confirm-0"));
    h.click("cloud-keep-0");
    assert!(
        h.is_laid_out("cloud-remove-0"),
        "keep withdraws the question"
    );

    h.click("cloud-remove-0");
    h.click("cloud-confirm-0");
    assert_eq!(
        backend.sent().last(),
        Some(&Command::CloudRemove {
            provider: CloudProvider::Aws,
            context: ContextName::new("eks-web"),
        })
    );
}

#[gpui::test]
fn scope_filters_narrow_the_rows(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    h.play(
        &backend,
        run_events(
            CloudProvider::Aws,
            ready(CloudProvider::Aws),
            prod.clone(),
            vec![eks(&prod, "web"), eks(&dev, "api")],
        ),
    );
    assert!(h.is_laid_out("cloud-filters"));
    assert!(h.is_laid_out("cloud-row-1"));
    h.click("cloud-filter-1");
    assert!(h.is_laid_out("cloud-row-0"));
    assert!(!h.is_laid_out("cloud-row-1"), "one scope is left");
    h.click("cloud-filter-all");
    assert!(h.is_laid_out("cloud-row-1"));
}

#[gpui::test]
fn a_degraded_provider_shows_its_hints_and_copies_the_fix(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    let hint = SetupHint::with_command(
        "the GKE auth plugin is not installed",
        "gcloud components install gke-gcloud-auth-plugin",
    );
    h.play(
        &backend,
        vec![CloudDiscoveryEvent::Detected(degraded(
            CloudProvider::Gcp,
            vec![hint],
        ))],
    );
    assert!(h.is_laid_out("cloud-card-status-gcp:Needs setup"));
    assert!(h.is_laid_out("cloud-banner-line-0"));
    h.click("cloud-banner-copy-0");
    let copied = h
        .vcx
        .read(|cx| cx.read_from_clipboard().and_then(|c| c.text()));
    assert_eq!(
        copied.as_deref(),
        Some("gcloud components install gke-gcloud-auth-plugin")
    );
    assert!(h.is_laid_out("cloud-notice"));
}

#[gpui::test]
fn every_error_kind_ends_in_words_and_a_fix_not_a_raw_failure(cx: &mut TestAppContext) {
    let cases = [
        (
            OxiError::auth("`aws` exit 255: Unable to locate credentials", false),
            "not logged in",
            true,
        ),
        (OxiError::forbidden("AccessDenied"), "not allowed", false),
        (
            OxiError::network("could not connect to the endpoint URL"),
            "could not be reached",
            false,
        ),
        (OxiError::timeout("exit 124"), "took too long", false),
        (
            OxiError::unsupported("`aws` was not found on PATH"),
            "was not found",
            false,
        ),
        (
            OxiError::unsupported("the auth plugin is missing"),
            "missing",
            false,
        ),
    ];
    for (error, words, has_command) in cases {
        let (mut h, backend) = Harness::scripted(cx);
        h.play(
            &backend,
            vec![
                CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws)),
                CloudDiscoveryEvent::Failed {
                    provider: CloudProvider::Aws,
                    error,
                },
            ],
        );
        assert!(h.is_laid_out("cloud-card-status-aws:Could not list"));
        assert!(h.is_laid_out("cloud-banner"));
        let text = h.read(|view| {
            view.model()
                .provider(CloudProvider::Aws)
                .error
                .clone()
                .expect("the failure has a remediation")
        });
        assert!(
            text.text.to_lowercase().contains(words),
            "{words:?} in {:?}",
            text.text
        );
        assert!(
            !text.text.contains("exit "),
            "no raw CLI output: {}",
            text.text
        );
        assert_eq!(h.is_laid_out("cloud-banner-copy-0"), has_command);
    }
}

#[gpui::test]
fn rapid_refresh_clicks_coalesce_into_one_run(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    h.play(
        &backend,
        vec![CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws))],
    );
    backend.hold(true);
    let before = backend.sent().len();
    for _ in 0..5 {
        h.update(|view, cx| view.refresh_provider(CloudProvider::Aws, cx));
    }
    h.vcx.run_until_parked();
    assert_eq!(backend.sent().len() - before, 1, "five clicks, one run");
    h.update(|view, cx| view.refresh_all(CloudRefreshMode::Force, cx));
    assert_eq!(
        backend.sent().len() - before,
        1,
        "refresh all waits for the run too"
    );
}

#[gpui::test]
fn escape_cancels_a_running_discovery_and_does_nothing_otherwise(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    h.play(
        &backend,
        vec![CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws))],
    );
    let focus = h.read(|view| view.focus.clone());
    h.vcx.update(|window, cx| window.focus(&focus, cx));

    let before = backend.sent().len();
    h.vcx.simulate_keystrokes("escape");
    assert_eq!(
        backend.sent().len(),
        before,
        "nothing runs, so nothing to cancel"
    );

    backend.hold(true);
    h.update(|view, cx| view.refresh_provider(CloudProvider::Aws, cx));
    h.vcx.run_until_parked();
    h.vcx.simulate_keystrokes("escape");
    assert_eq!(
        backend.sent().last(),
        Some(&Command::CloudCancel { provider: None })
    );
}

#[gpui::test]
fn the_refresh_key_forces_a_run_of_every_provider(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    h.play(
        &backend,
        vec![CloudDiscoveryEvent::Detected(ready(CloudProvider::Aws))],
    );
    let focus = h.read(|view| view.focus.clone());
    h.vcx.update(|window, cx| window.focus(&focus, cx));
    h.vcx.simulate_keystrokes("secondary-r");
    assert_eq!(
        backend.sent().last(),
        Some(&Command::CloudRefresh {
            provider: None,
            mode: CloudRefreshMode::Force,
        })
    );
}

#[gpui::test]
fn the_view_sets_the_catalog_cloud_key_context_and_flags_loading(cx: &mut TestAppContext) {
    use oxikube_keymap::KeyContextual as _;
    let (mut h, backend) = Harness::scripted(cx);
    assert_eq!(CloudView::KEY_CONTEXT, "CatalogCloud");
    let idle = h.read(|view| view.key_context());
    assert!(idle.contains("CatalogCloud") && !idle.contains("Loading"));
    backend.hold(true);
    h.update(|view, cx| view.refresh_provider(CloudProvider::Aws, cx));
    let busy = h.read(|view| view.key_context());
    assert!(busy.contains("CatalogCloud") && busy.contains("Loading"));
}

#[gpui::test]
fn closing_the_section_cancels_the_running_discovery(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = oxikube_workspace::test_support::open_workspace(cx);
    vcx.update(|_, cx| oxikube_runtime::init_deterministic(cx));
    let backend = ScriptedBackend::new();
    backend.hold(true);
    let deps = CloudDeps {
        backend: std::rc::Rc::new(backend.clone()),
        clock: std::sync::Arc::new(oxikube_testkit::FakeClockPort::new(
            jiff::Timestamp::from_second(1_800_000_000).expect("a valid timestamp"),
        )),
    };
    vcx.update(|_, cx| {
        let view = cx.new(|cx| CloudView::new(deps, cx));
        view.update(cx, |view, cx| view.refresh_provider(CloudProvider::Aws, cx));
        drop(view);
    });
    vcx.run_until_parked();
    assert!(
        backend
            .sent()
            .contains(&Command::CloudCancel { provider: None }),
        "{:?}",
        backend.sent()
    );
    drop(workspace);
}

#[gpui::test]
fn the_age_of_the_listing_reads_on_the_card(cx: &mut TestAppContext) {
    let (mut h, backend) = Harness::scripted(cx);
    let scope = aws_scope("prod", "eu-west-1");
    let fetched = h.clock.now() - std::time::Duration::from_secs(4 * 60);
    h.play(
        &backend,
        vec![
            CloudDiscoveryEvent::Cached {
                provider: CloudProvider::Aws,
                detection: Some(ready(CloudProvider::Aws)),
                clusters: vec![eks(&scope, "web")],
                fetched_at: Some(fetched),
                stale: false,
            },
            CloudDiscoveryEvent::Finished {
                provider: CloudProvider::Aws,
                clusters: vec![eks(&scope, "web")],
                failed_scopes: 0,
            },
        ],
    );
    let at = h.read(|view| view.model().provider(CloudProvider::Aws).fetched_at);
    assert_eq!(at, Some(fetched));
    assert!(h.is_laid_out("cloud-row-0"), "the cache shows at once");
    assert!(h.is_laid_out("cloud-card-meta-aws:aws 2.15.0 · refreshed 4m ago"));
}
