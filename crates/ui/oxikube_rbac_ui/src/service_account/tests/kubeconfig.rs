//! The Kubeconfig section: choices, the request, the masked preview, copy, save and discard.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{Modifiers, TestAppContext};
use oxikube_app::sa_kubeconfig::{GeneratedKubeconfig, KubeconfigVault, MASK, TokenSource};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::secret::SecretText;
use oxikube_testkit::FakeFsPort;
use oxikube_testkit::gpui_test::{TestApp, TestWindow};
use oxikube_workspace::CommandDispatcher;
use serde_json::json;

use super::{cluster, service_account};
use crate::service_account::{
    BoundRoles, Expiry, KubeconfigDeps, KubeconfigState, ServiceAccountFacts, ServiceAccountPanel,
};

const TOKEN: &str = "eyJhbGciOiJSUzI1NiJ9.sa-token-payload.signature";

#[derive(Clone, Default)]
struct Bus(Rc<RefCell<Vec<Command>>>);

impl CommandDispatcher for Bus {
    fn dispatch(&self, command: Command, _: &mut gpui::App) {
        self.0.borrow_mut().push(command);
    }
}

fn target() -> ResourceRef {
    ResourceRef::namespaced(
        cluster(),
        Gvk::new("", "v1", "ServiceAccount"),
        "shop",
        "ci",
    )
}

fn generated(source: TokenSource) -> GeneratedKubeconfig {
    let warning = source
        .is_long_lived()
        .then(|| "WARNING: this token does not expire.".to_owned());
    GeneratedKubeconfig {
        service_account: target(),
        text: SecretText::new(format!(
            "# kubeconfig\nusers:\n- user:\n    token: {TOKEN}\n"
        )),
        preview: format!("# kubeconfig\nusers:\n- user:\n    token: {MASK}\n"),
        source,
        expires_at: Some("2026-10-28T13:00:00Z".into()),
        warning,
    }
}

struct Rig {
    window: TestWindow<ServiceAccountPanel>,
    bus: Bus,
    vault: KubeconfigVault,
    fs: Arc<FakeFsPort>,
}

impl Rig {
    fn new(cx: &mut TestAppContext) -> Self {
        cx.update(oxikube_ui::init);
        let vault = KubeconfigVault::new();
        let fs = Arc::new(FakeFsPort::new());
        cx.update(|cx| {
            oxikube_runtime::init_deterministic(cx);
            cx.set_global(KubeconfigDeps {
                fs: fs.clone(),
                vault: vault.clone(),
            });
        });
        let bus = Bus::default();
        let facts = ServiceAccountFacts::from_resource(&service_account(json!({})));
        let dispatcher: Rc<dyn CommandDispatcher> = Rc::new(bus.clone());
        let mut app = TestApp::new(cx);
        let mut window = app.open_window(move |_, _| {
            ServiceAccountPanel::fixed(target(), facts, BoundRoles::Loading, dispatcher)
        });
        window.draw_frame();
        Self {
            window,
            bus,
            vault,
            fs,
        }
    }

    fn shown(&mut self, selector: &'static str) -> bool {
        self.window.bounds(selector).is_some()
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .window
            .bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        self.window
            .simulate_click(bounds.center(), Modifiers::default());
        self.window.draw_frame();
    }

    /// The vault gets a kubeconfig and the panel looks.
    fn deliver(&mut self, kubeconfig: GeneratedKubeconfig) {
        self.vault.put(kubeconfig);
        self.window
            .update_root(|panel, _, cx| panel.refresh_kubeconfig_for_test(cx));
        self.window.draw_frame();
    }

    fn state<R>(&mut self, read: impl FnOnce(&KubeconfigState) -> R) -> R {
        self.window.read_root(|panel, _| read(&panel.kubeconfig))
    }
}

#[gpui::test]
fn the_section_offers_the_lifetimes_and_a_button_but_no_result_yet(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    for selector in [
        "sa-kubeconfig",
        "sa-kubeconfig-expiry-600",
        "sa-kubeconfig-expiry-3600",
        "sa-kubeconfig-expiry-28800",
        "sa-kubeconfig-expiry-86400",
        "sa-kubeconfig-expiry-604800",
        "sa-kubeconfig-legacy",
        "sa-kubeconfig-generate",
    ] {
        assert!(rig.shown(selector), "{selector} is not drawn");
    }
    assert!(!rig.shown("sa-kubeconfig-preview"));
    assert!(!rig.shown("sa-kubeconfig-copy"));
    assert_eq!(rig.state(|state| state.expiry), Expiry::OneHour);
}

#[gpui::test]
fn generate_sends_the_command_with_the_chosen_lifetime(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.click("sa-kubeconfig-expiry-28800");
    rig.click("sa-kubeconfig-generate");
    let sent = rig.bus.0.borrow().clone();
    assert_eq!(
        sent,
        [Command::ServiceAccountGenerateKubeconfig {
            target: target(),
            expiration_seconds: Some(28_800),
            audiences: Vec::new(),
            allow_legacy_token: false,
        }]
    );
    assert!(rig.state(|state| state.waiting), "it waits for the result");
    assert!(rig.shown("sa-kubeconfig-waiting"));
}

#[gpui::test]
fn the_legacy_choice_travels_with_the_command(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.window
        .update_root(|panel, _, cx| panel.set_kubeconfig_allow_legacy(true, cx));
    rig.click("sa-kubeconfig-generate");
    let sent = rig.bus.0.borrow().clone();
    assert!(matches!(
        &sent[..],
        [Command::ServiceAccountGenerateKubeconfig {
            allow_legacy_token: true,
            expiration_seconds: Some(3600),
            ..
        }]
    ));
}

#[gpui::test]
fn a_result_in_the_vault_shows_a_masked_preview_with_its_actions(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::TokenRequest));
    for selector in [
        "sa-kubeconfig-preview",
        "sa-kubeconfig-copy",
        "sa-kubeconfig-save",
        "sa-kubeconfig-reveal",
        "sa-kubeconfig-discard",
        "sa-kubeconfig-expiry-note",
    ] {
        assert!(rig.shown(selector), "{selector} is not drawn");
    }
    assert!(
        !rig.shown("sa-kubeconfig-warning"),
        "a short-lived token has no warning"
    );
    assert!(!rig.shown("sa-kubeconfig-waiting"));
    assert!(!rig.state(|state| state.reveal), "masked by default");
}

#[gpui::test]
fn a_long_lived_token_is_shown_with_its_warning(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::LegacySecret));
    assert!(rig.shown("sa-kubeconfig-warning"));
}

#[gpui::test]
fn show_token_reveals_and_hide_masks_again(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::TokenRequest));
    rig.click("sa-kubeconfig-reveal");
    assert!(rig.state(|state| state.reveal));
    rig.click("sa-kubeconfig-reveal");
    assert!(!rig.state(|state| state.reveal));
}

#[gpui::test]
fn copy_puts_the_whole_file_on_the_clipboard(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::TokenRequest));
    rig.click("sa-kubeconfig-copy");
    let copied = rig
        .window
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("something was copied");
    assert!(copied.contains(TOKEN), "the real token, not the mask");
    assert!(!copied.contains(MASK));
    assert!(rig.state(|state| state.note.is_some()));
}

#[gpui::test]
fn save_asks_where_and_writes_an_owner_only_file(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::TokenRequest));
    rig.click("sa-kubeconfig-save");
    assert!(rig.window.did_prompt_for_new_path());
    let path = PathBuf::from("/home/me/ci.kubeconfig");
    let chosen = path.clone();
    rig.window
        .simulate_new_path_selection(move |_| Some(chosen));
    rig.window.run_until_parked();
    rig.window
        .advance_clock(std::time::Duration::from_millis(20));
    let written = rig.fs.file(&path).expect("the file was written");
    assert!(String::from_utf8(written).unwrap().contains(TOKEN));
    assert!(rig.fs.is_private(&path), "mode 0600");
}

#[gpui::test]
fn cancelling_the_save_panel_writes_nothing(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.deliver(generated(TokenSource::TokenRequest));
    rig.click("sa-kubeconfig-save");
    rig.window.simulate_new_path_selection(|_| None);
    rig.window.run_until_parked();
    assert!(rig.fs.file("/home/me/ci.kubeconfig").is_none());
}

#[gpui::test]
fn discard_drops_the_token_here_and_in_the_vault(cx: &mut TestAppContext) {
    let mut rig = Rig::new(cx);
    rig.click("sa-kubeconfig-generate");
    rig.deliver(generated(TokenSource::TokenRequest));
    assert_eq!(rig.vault.len(), 1);
    rig.click("sa-kubeconfig-discard");
    assert!(rig.vault.is_empty());
    assert!(!rig.shown("sa-kubeconfig-preview"));
    assert!(rig.state(|state| state.generated.is_none()));
}

#[test]
fn a_closed_panel_leaves_no_token_in_the_vault() {
    let vault = KubeconfigVault::new();
    vault.put(generated(TokenSource::TokenRequest));
    let mut state = KubeconfigState::default();
    state.asked_for(target(), vault.clone());
    drop(state);
    assert!(vault.is_empty());
}

#[test]
fn the_choices_cover_ten_minutes_to_a_week() {
    let seconds: Vec<u32> = Expiry::ALL.iter().map(|expiry| expiry.seconds()).collect();
    assert_eq!(seconds, [600, 3_600, 28_800, 86_400, 604_800]);
    assert!(seconds[0] >= oxikube_app::sa_kubeconfig::MIN_EXPIRATION_SECONDS);
}
