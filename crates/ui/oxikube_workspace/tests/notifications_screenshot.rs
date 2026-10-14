//! The notification centre (E14-S05) in light and dark: the right-dock panel with notifications of
//! every severity (unread and read, a grouped burst), the status-bar bell with its count, and a
//! warning toast.
//!
//! Same harness and gate as `screenshot.rs` (`harness = false`, needs a GPU device, feature
//! `screenshot`, nightly job): `cargo test -p oxikube_workspace --features screenshot --test
//! notifications_screenshot`. Goldens live in `tests/goldens/<os>/notifications_{dark,light}.png`;
//! regenerate with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::{path::Path, process::ExitCode, rc::Rc, sync::Arc};

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_app::events::{EventService, EventsConfig};
use oxikube_app::notifications::{GroupKey, NotificationInput, NotificationService};
use oxikube_app::{ClusterSessionManager, session::SessionOptions};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{ClusterContext, NotificationLevel, SourceId};
use oxikube_testkit::{
    FakeClockPort, FakeClusterConnectorPort, FakeClusterSourcePort, FakeStatePort,
    headless::HEADLESS_SCALE_FACTOR,
    screenshot::{RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path},
};
use oxikube_ui::{Tokens, root::new_root};
use oxikube_workspace::notifications::{NotificationDeps, NotificationPanel, install};
use oxikube_workspace::{CommandDispatcher, Workspace, test_support::TestItem};

const WIDTH: f32 = 900.0;
const HEIGHT: f32 = 520.0;

struct NoBus;

impl CommandDispatcher for NoBus {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn input(
    cluster: &ClusterId,
    level: NotificationLevel,
    name: &str,
    reason: &str,
    body: &str,
) -> NotificationInput {
    NotificationInput {
        cluster: cluster.clone(),
        level,
        title: format!("Pod default/{name}: {reason}"),
        body: body.to_owned(),
        target: Some(ResourceRef::new(
            cluster.clone(),
            Gvk::new("", "v1", "Pod"),
            Some("default".into()),
            name,
        )),
        dedupe_key: format!("event/{cluster}/{name}/{reason}").into(),
        group: Some(GroupKey {
            reason: reason.into(),
            noun: "pod".into(),
        }),
        occurrences: 1,
    }
}

/// The centre of one cluster, with a mix of severities, a burst and a read row.
struct Fixture {
    service: NotificationService,
    sessions: ClusterSessionManager,
    cluster: ClusterId,
    clock: Arc<FakeClockPort>,
}

fn centre() -> anyhow::Result<Fixture> {
    let cluster = ClusterId::new("/home/me/.kube/config", &ContextName::new("prod-eu"));
    let context = ClusterContext::new(
        cluster.clone(),
        ContextName::new("prod-eu"),
        SourceId("kubeconfig".into()),
    );
    let clock = Arc::new(FakeClockPort::default());
    let sessions = ClusterSessionManager::new(
        Arc::new(FakeClusterConnectorPort::new()),
        Arc::new(FakeClusterSourcePort::new().with_contexts([context.clone()])),
        clock.clone(),
    );
    sessions.open(&context, SessionOptions::default());
    let events = EventService::new(sessions.clone(), clock.clone(), EventsConfig::default());
    let service = NotificationService::new(
        events,
        sessions.clone(),
        Arc::new(FakeStatePort::new()),
        clock.clone(),
    );
    let read = service
        .submit(input(
            &cluster,
            NotificationLevel::Info,
            "db-0",
            "Pulled",
            "Container image already present on machine",
        ))
        .ok_or_else(|| anyhow::anyhow!("listed"))?;
    service.mark_read(read.id);
    service.submit(input(
        &cluster,
        NotificationLevel::Error,
        "api-7",
        "OOMKilled",
        "Container api was killed: out of memory",
    ));
    for name in ["web-0", "web-1", "web-2", "web-3", "web-4", "web-5"] {
        service.submit(input(
            &cluster,
            NotificationLevel::Warning,
            name,
            "BackOff",
            "Back-off restarting failed container",
        ));
    }
    service.submit(input(
        &cluster,
        NotificationLevel::Success,
        "batch-1",
        "Completed",
        "Job finished",
    ));
    Ok(Fixture {
        service,
        sessions,
        cluster,
        clock,
    })
}

fn render(tokens: Tokens) -> anyhow::Result<RgbaImage> {
    let Fixture {
        service,
        sessions,
        cluster,
        clock,
    } = centre()?;
    let deps = NotificationDeps {
        service: service.clone(),
        sessions,
        dispatcher: Rc::new(NoBus),
    };
    let mut cx = headless();
    // The wiring must outlive the capture (the toasts follow through it) and be dropped before
    // the app, or the leak detector fails the run.
    let kept = Rc::new(std::cell::RefCell::new(None));
    let keep = kept.clone();
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |window, cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, tokens);
        cx.set_reduce_motion(true);
        let workspace = cx.new(|cx| Workspace::new(window, cx));
        let tab = cx.new(|cx| TestItem::new("prod-eu", cx));
        workspace.update(cx, |ws, cx| ws.open_item(tab, window, cx));
        let wiring = install(&workspace, deps, window, cx);
        // The shown cluster tab: the header offers to mute it.
        let active = cluster.clone();
        wiring
            .panel
            .update(cx, |panel, cx| panel.set_active_cluster(Some(active), cx));
        workspace.update(cx, |ws, cx| {
            ws.toggle_panel::<NotificationPanel>(window, cx);
        });
        keep.borrow_mut().replace(wiring);
        new_root(workspace, window, cx)
    })?;
    // A new warning once the raise window passed: it toasts.
    clock.advance(std::time::Duration::from_secs(60));
    service.submit(input(
        &cluster,
        NotificationLevel::Warning,
        "cache-0",
        "FailedMount",
        "MountVolume.SetUp failed for volume \"data\": timed out",
    ));
    cx.run_until_parked();
    // The centre redraws through `notify_coalesced`, delivered on the next presented frame (or a
    // backstop timer): a headless window presents none, so deliver it now.
    let centre = kept
        .borrow()
        .as_ref()
        .map(|wiring: &oxikube_workspace::notifications::NotificationWiring| wiring.centre.clone())
        .ok_or_else(|| anyhow::anyhow!("mounted"))?;
    cx.update(|cx| centre.update(cx, |_, cx| cx.notify()));
    // Let the rows and the toast land (the coalesced redraw's backstop timer included).
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| window.draw(cx).clear(cx))?;
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into());
    drop(kept.borrow_mut().take());
    cx.update_window(window.into(), |_, window, _| window.remove_window())?;
    cx.run_until_parked();
    image
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
    check("notifications_dark", render(Tokens::dark())?)?;
    check("notifications_light", render(Tokens::light())?)
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => {
            println!("notifications screenshot: ok");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("notifications screenshot failed: {err:#}");
            ExitCode::FAILURE
        }
    }
}
