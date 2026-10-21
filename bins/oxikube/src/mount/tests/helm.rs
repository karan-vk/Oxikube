//! Helm capability gating in the running app (E16-S11): `helm::Recheck` and `helm::LocateBinary`
//! on the bus with their tool stubs, the gate following detection, the palette's capabilities
//! following the gate, and the sidebar's Helm entry opening the Helm page with its banner. The
//! binary itself is not run here (the test runtime has no tokio reactor); the controller's own
//! tests run a fake `helm` on the real runtime.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use futures::executor::block_on;
use gpui::TestAppContext;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::{Command, CommandId};
use oxikube_domain::helm::{GateReason, HelmCapabilities, HelmGate, HelmVersion};
use oxikube_helm_ui::{HelmGateState, HelmPage};
use oxikube_testkit::TestPorts;

use super::App;
use crate::app_state::AppState;
use crate::mount::helm::FileOpener;
use crate::startup::ConfigSource;

fn bus(app: &mut App) -> oxikube_app::command_bus::CommandBus {
    let state = app.vcx.update(|_, cx| AppState::global(cx));
    state.command_bus().expect("the bus").clone()
}

fn run(app: &mut App, command: Command) -> oxikube_app::command_bus::CommandOutput {
    let bus = bus(app);
    let outcome = block_on(bus.dispatch(command, DispatchContext::new(Initiator::Ui, "me")));
    app.vcx.run_until_parked();
    outcome
        .expect("the command runs")
        .completed()
        .expect("it completes")
}

#[gpui::test]
fn the_helm_commands_are_on_the_bus_with_tool_stubs(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let bus = bus(&mut app);
    for (id, tool) in [
        (CommandId::HELM_RECHECK, "helm.recheck"),
        (CommandId::HELM_LOCATE_BINARY, "helm.locate_binary"),
    ] {
        assert_eq!(bus.owner(id), Some("oxikube"), "{id}");
        assert_eq!(bus.tool(id).map(|t| t.name.as_str()), Some(tool), "{id}");
    }
}

#[gpui::test]
fn start_up_detects_in_the_background_and_a_recheck_runs_again(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    // Detection has reported (the deterministic runtime finds no binary): no longer "looking".
    let gate = app.vcx.update(|_, cx| HelmGateState::current(cx));
    assert_eq!(
        gate,
        HelmGate::Unavailable {
            reason: GateReason::BinaryMissing
        }
    );
    let output = run(&mut app, Command::HelmRecheck);
    assert!(output.message.unwrap().contains("Helm"));
}

#[gpui::test]
fn locate_creates_and_shows_settings_json(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().to_path_buf();
    let mut app = App::start_with_env(
        cx,
        TestPorts::seeded(),
        |env| env.config = ConfigSource::Dir(config),
        |_| {},
    );
    let opened = Rc::new(RefCell::new(Vec::<PathBuf>::new()));
    let seen = opened.clone();
    app.vcx.update(|_, cx| {
        cx.set_global(FileOpener(Rc::new(move |path, _| {
            seen.borrow_mut().push(path.to_path_buf());
        })));
    });
    let output = run(&mut app, Command::HelmLocateBinary);
    let file = dir.path().join("settings.json");
    assert!(file.exists());
    assert_eq!(*opened.borrow(), std::slice::from_ref(&file));
    let data = output.data.unwrap();
    assert_eq!(data["setting"], "helm.path");
    assert_eq!(data["path"], file.display().to_string());
}

#[gpui::test]
fn locate_without_a_config_directory_says_so(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    let bus = bus(&mut app);
    let outcome = block_on(bus.dispatch(
        Command::HelmLocateBinary,
        DispatchContext::new(Initiator::Ui, "me"),
    ));
    let error = outcome.expect_err("nothing to open");
    assert!(error.to_string().contains("settings.json"), "{error}");
}

#[gpui::test]
fn the_sidebar_entry_opens_the_helm_page_and_the_banner_follows_the_gate(cx: &mut TestAppContext) {
    let mut app = App::start(cx, TestPorts::seeded());
    app.press("enter");
    app.tick();
    app.click("sidebar-entry-helm/releases");
    app.tick();
    let tab = app.cluster_tabs().pop().expect("the cluster tab");
    let workspace = app.vcx.update(|_, cx| tab.read(cx).workspace().clone());
    let pages = app
        .vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<HelmPage>());
    assert_eq!(pages.len(), 1, "the Helm entry opened the page");
    assert!(app.drawn("helm-page"));
    assert!(app.drawn("helm-banner"), "no helm binary: guidance shows");
    assert!(app.drawn("helm-locate"));

    // A binary appears (a recheck found it): the banner goes and the page lists everything.
    app.vcx.update(|_, cx| {
        let state = HelmGateState::global(cx).expect("the gate");
        state.update(cx, |state, cx| {
            state.set_cli_capabilities(HelmCapabilities::full(HelmVersion::new(3, 14, 2)), cx);
        });
    });
    app.vcx.run_until_parked();
    assert!(!app.drawn("helm-banner"));
    assert!(app.drawn("helm-feature-install"));
}

#[gpui::test]
fn the_palette_sees_the_helm_capability_only_while_the_gate_is_full(cx: &mut TestAppContext) {
    use oxikube_domain::Capabilities;
    let session = Capabilities::MUTATE | Capabilities::HELM;
    let without = crate::mount::helm::palette_capabilities(&HelmGate::default(), session);
    assert_eq!(without, Capabilities::MUTATE);
    let with = crate::mount::helm::palette_capabilities(&HelmGate::Full, Capabilities::MUTATE);
    assert_eq!(with, Capabilities::MUTATE | Capabilities::HELM);
    let _ = cx;
}
