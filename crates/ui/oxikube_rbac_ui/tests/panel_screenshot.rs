//! Screenshot of the ServiceAccount detail panel (E17-S04): an account with a token Secret, an
//! image pull secret, the automount flag unset, and three bound roles (a namespaced grant, a
//! cluster-wide grant and one reached through a group). Rendered through
//! `Window::render_to_image`, so the pixels are the real view's output.
//!
//! `harness = false`: on macOS the platform text system can only be created on the process main
//! thread. Needs a GPU device, so it only builds with `--features screenshot` and runs in the
//! nightly job: `cargo test -p oxikube_rbac_ui --features screenshot --test panel_screenshot`.
//! Regenerate the golden with `OXIKUBE_UPDATE_GOLDENS=1`.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "a harness-less test binary reports its results on stdout and stderr"
)]

use std::path::Path;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{AppContext as _, HeadlessAppContext, px, size};
use oxikube_domain::command::Command;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_domain::rbac::{BindingKind, BindingRef, GrantScope, RoleKind, RoleRef};
use oxikube_rbac_ui::service_account::{
    Automount, BoundRole, BoundRoles, ServiceAccountFacts, ServiceAccountPanel,
};
use oxikube_testkit::headless::HEADLESS_SCALE_FACTOR;
use oxikube_testkit::screenshot::{
    RgbaImage, Tolerance, check_golden, distinct_colors_at_least, golden_path,
};
use oxikube_workspace::CommandDispatcher;

const WIDTH: f32 = 760.0;
const HEIGHT: f32 = 420.0;

struct Nowhere;

impl CommandDispatcher for Nowhere {
    fn dispatch(&self, _: Command, _: &mut gpui::App) {}
}

fn headless() -> HeadlessAppContext {
    let text_system = gpui_platform::current_platform(true).text_system();
    HeadlessAppContext::with_platform(text_system, Arc::new(oxikube_ui::Assets), || {
        gpui_platform::current_headless_renderer()
    })
}

fn bound(
    kind: BindingKind,
    namespace: Option<&str>,
    name: &str,
    (role_kind, role): (RoleKind, &str),
    via_group: Option<&str>,
) -> BoundRole {
    BoundRole {
        binding: BindingRef {
            kind,
            namespace: namespace.map(Arc::from),
            name: name.into(),
        },
        role: RoleRef {
            kind: role_kind,
            name: role.into(),
        },
        scope: namespace.map_or(GrantScope::Cluster, |ns| GrantScope::Namespace(ns.into())),
        via_group: via_group.map(Arc::from),
        role_found: true,
    }
}

fn render() -> anyhow::Result<RgbaImage> {
    let mut cx = headless();
    let cluster = ClusterId::new("kubeconfig.yaml", &ContextName::new("kind-oxikube"));
    let target =
        ResourceRef::namespaced(cluster, Gvk::new("", "v1", "ServiceAccount"), "shop", "ci");
    let facts = ServiceAccountFacts {
        namespace: "shop".into(),
        name: "ci".into(),
        secrets: vec!["ci-token".into()],
        image_pull_secrets: vec!["registry".into()],
        automount: Automount::Unset,
    };
    let roles = BoundRoles::Known {
        roles: vec![
            bound(
                BindingKind::RoleBinding,
                Some("shop"),
                "ci-edit",
                (RoleKind::ClusterRole, "edit"),
                None,
            ),
            bound(
                BindingKind::ClusterRoleBinding,
                None,
                "ci-nodes",
                (RoleKind::ClusterRole, "node-reader"),
                None,
            ),
            bound(
                BindingKind::ClusterRoleBinding,
                None,
                "discovery",
                (RoleKind::ClusterRole, "system:discovery"),
                Some("system:authenticated"),
            ),
        ],
        missing: Vec::new(),
    };
    let window = cx.open_window(size(px(WIDTH), px(HEIGHT)), |_, cx| {
        oxikube_ui::init(cx);
        // Pin the appearance: `init` follows the system, which differs between machines.
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        cx.set_reduce_motion(true);
        cx.new(|_| ServiceAccountPanel::fixed(target, facts, roles, Rc::new(Nowhere)))
    })?;
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

fn main() -> ExitCode {
    let name = "service_account_panel_dark";
    if let Err(err) = render().and_then(|image| check(name, image)) {
        eprintln!("{name} screenshot failed: {err:#}");
        return ExitCode::FAILURE;
    }
    println!("service account panel screenshot: ok");
    ExitCode::SUCCESS
}
