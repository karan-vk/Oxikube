//! Cloud section screenshots, rendered through `Window::render_to_image` (E18-S06).
//!
//! - `cloud_loaded`: AWS and Google Cloud detected, EKS clusters listed in two scopes with one
//!   already in the catalog.
//! - `cloud_loading`: a run in progress, "Listing 1 of 3 scopes", results arriving.
//! - `cloud_degraded`: the GKE auth plugin and the login are missing; the banner says what to run.
//!
//! `harness = false` and `--features screenshot`, like the other screenshot tests of this crate
//! (see `catalog_screenshot.rs` for why). Checks the image size, that it is not blank and, when a
//! golden exists for this OS, that it matches within tolerance. Regenerate with
//! `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a screenshot test reports on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::cloud::CloudDiscoveryEvent;
use oxikube_catalog_ui::cloud::test_support::{ScriptedBackend, aws_scope, eks, managed};
use oxikube_catalog_ui::cloud::{CloudDeps, CloudView};
use oxikube_domain::cloud::{
    CloudDetection, CloudProvider, CloudToolStatus, DiscoveredCluster, SetupHint,
};
use oxikube_testkit::FakeClockPort;
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};

const WIDTH: f32 = 1000.0;
const HEIGHT: f32 = 520.0;
const NOW: i64 = 1_800_000_000;

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn detection(provider: CloudProvider, hints: Vec<SetupHint>) -> CloudDetection {
    CloudDetection {
        provider,
        status: if hints.is_empty() {
            CloudToolStatus::Ready
        } else {
            CloudToolStatus::NotAuthenticated
        },
        binary: Some(format!("/usr/local/bin/{}", provider.cli()).into()),
        version: Some("2.15.30".into()),
        hints,
    }
}

fn started(
    provider: CloudProvider,
    scopes: Vec<oxikube_domain::cloud::CloudScope>,
) -> CloudDiscoveryEvent {
    CloudDiscoveryEvent::Started { provider, scopes }
}

/// One scenario: the events played to the view, whether the run is still going, and what is
/// already in the catalog.
struct Scenario {
    events: Vec<CloudDiscoveryEvent>,
    running: bool,
    added: Vec<DiscoveredCluster>,
}

fn loaded() -> Scenario {
    let (prod, dev) = (
        aws_scope("prod", "eu-west-1"),
        aws_scope("dev", "us-east-1"),
    );
    let clusters = vec![
        eks(&prod, "payments"),
        eks(&prod, "search"),
        eks(&dev, "sandbox"),
    ];
    Scenario {
        events: vec![
            CloudDiscoveryEvent::Detected(detection(CloudProvider::Aws, Vec::new())),
            CloudDiscoveryEvent::Detected(detection(CloudProvider::Gcp, Vec::new())),
            started(CloudProvider::Aws, vec![prod, dev]),
            CloudDiscoveryEvent::Finished {
                provider: CloudProvider::Aws,
                clusters: clusters.clone(),
                failed_scopes: 0,
            },
        ],
        running: false,
        added: vec![clusters[0].clone()],
    }
}

fn loading() -> Scenario {
    let scopes: Vec<_> = ["eu-west-1", "us-east-1", "ap-south-1"]
        .iter()
        .map(|region| aws_scope("prod", region))
        .collect();
    Scenario {
        events: vec![
            CloudDiscoveryEvent::Detected(detection(CloudProvider::Aws, Vec::new())),
            started(CloudProvider::Aws, scopes.clone()),
            CloudDiscoveryEvent::ScopeDone {
                scope: scopes[0].clone(),
                clusters: vec![eks(&scopes[0], "payments"), eks(&scopes[0], "search")],
            },
        ],
        running: true,
        added: Vec::new(),
    }
}

fn degraded() -> Scenario {
    let hints = vec![
        SetupHint::with_command(
            "the GKE auth plugin is not installed",
            "gcloud components install gke-gcloud-auth-plugin",
        ),
        SetupHint::with_command("not logged in to gcloud", "gcloud auth login"),
    ];
    Scenario {
        events: vec![CloudDiscoveryEvent::Detected(detection(
            CloudProvider::Gcp,
            hints,
        ))],
        running: false,
        added: Vec::new(),
    }
}

fn render(scenario: Scenario) -> anyhow::Result<RgbaImage> {
    let backend = ScriptedBackend::new();
    backend.hold(scenario.running);
    backend.set_added(scenario.added.iter().map(managed).collect());
    let mut cx = headless();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        oxikube_runtime::init_deterministic(cx);
        let deps = CloudDeps {
            backend: Rc::new(backend.clone()),
            clock: Arc::new(FakeClockPort::new(
                jiff::Timestamp::from_second(NOW).expect("a valid timestamp"),
            )),
        };
        let view = cx.new(|cx| CloudView::new(deps, cx));
        oxikube_ui::root::new_root(view, window, cx)
    })?;
    cx.run_until_parked();
    for event in scenario.events {
        backend.emit(event);
    }
    cx.run_until_parked();
    // The view redraws at frame cadence: let the coalesced redraw through.
    cx.advance_clock(oxikube_runtime::FRAME_INTERVAL * 2);
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())
}

fn check(name: &str, image: RgbaImage) -> anyhow::Result<()> {
    let scale = HEADLESS_SCALE_FACTOR;
    anyhow::ensure!(
        image.dimensions() == (WIDTH as u32 * scale, HEIGHT as u32 * scale),
        "unexpected image size {:?}",
        image.dimensions()
    );
    anyhow::ensure!(
        distinct_colors_at_least(&image, 8),
        "{name}: frame looks blank"
    );
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let golden = golden_path(&goldens, name);
    let updating =
        std::env::var_os("OXIKUBE_UPDATE_GOLDENS").is_some_and(|v| v != "0" && !v.is_empty());
    if golden.exists() || updating {
        check_golden(&image, &golden, Tolerance::default())?;
        println!("{name} matches {}", golden.display());
    } else {
        println!(
            "no golden for {} yet; structural checks only",
            std::env::consts::OS
        );
    }
    Ok(())
}

fn run() -> anyhow::Result<()> {
    check("cloud_loaded", render(loaded())?)?;
    check("cloud_loading", render(loading())?)?;
    check("cloud_degraded", render(degraded())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("cloud screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("cloud screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
