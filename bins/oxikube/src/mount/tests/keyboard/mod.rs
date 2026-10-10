//! The keyboard suite of the command palette, the `:` jump bar and the vim keymap (E11-S12).
//!
//! Palette, jump bar and keymaps are driven by keys, which is what unit tests of one view miss:
//! the binding, the key context, the focus path, the modal layer, the `CommandBus` and the view
//! that ends up on screen all have to agree. Every scenario here presses keys into the app's real
//! main window (`startup::init` + `open_main_window` over testkit fakes, the same path as
//! `mount/tests`) and asserts on what the app did: the commands the bus ran (its audit log), the
//! tables the cluster tab shows with their rows, the calls that reached the cluster port.
//!
//! The suite lives in `bins/oxikube`, not in `oxikube_palette`: only the binary mounts the palette
//! host, the jump host, the resource views and the bus together, and `oxikube_palette` cannot
//! depend on `oxikube_resources_ui` (which depends on it). The palette's own `#[gpui::test]`s
//! (`oxikube_palette::command_palette::tests`, `jump::tests`) cover each view over stand-ins.
//!
//! | File | Scenario |
//! |---|---|
//! | `palette` | A: open the palette, type, confirm; a mutating command hidden by a read-only session; every bus command listed |
//! | `colon` | B: `:pods kube-system`, `:deploy`, `:pod app=nginx`, a CRD alias, an unknown alias |
//! | `vim` | C: `base_keymap: vim` in the table: `j j k g g shift-g`, `d d` meets the guard, `/` and `:`; hot reload |
//! | `history` | D: `-`, `[`, `]` after a few jumps |
//! | `coverage` | the meta-test: each E11 acceptance scenario has a test here |
//!
//! # Failing with context
//!
//! A keymap regression is quickest to find with the key-context stack the keys were pressed in.
//! [`App::diagnostics`] describes it (the focused view is the innermost key context, then the
//! stack, the modal on top, the table's cursor) and `check!` appends it to a failed assertion.
//!
//! # Determinism
//!
//! No timers and no threads: the apps' fakes answer from the test executor, and time moves only
//! with [`App::tick`] (the GPUI test clock). Run the suite many times with
//! `cargo test -p oxikube keyboard -- --test-threads=1` in a loop to check for flakes.

mod colon;
mod coverage;
mod history;
mod palette;
mod vim;

use gpui::{BorrowAppContext as _, Entity, TestAppContext};
use oxikube_domain::Resource;
use oxikube_domain::ids::{Gvk, ResourceRef};
use oxikube_domain::kinds::{ResourceKind, VerbSet};
use oxikube_palette::CommandPalette;
use oxikube_palette::jump::JumpBar;
use oxikube_resources_ui::actions::DeleteDialog;
use oxikube_resources_ui::table::ResourceTable;
use oxikube_settings::SettingsStore;
use oxikube_testkit::{TestPorts, deployment, pod};

use super::App;
use crate::app_state::AppState;

/// Asserts `cond`; when it fails, the message is followed by [`App::diagnostics`].
macro_rules! check {
    ($app:expr, $cond:expr, $($message:tt)+) => {
        if !$cond {
            let diagnostics = $app.diagnostics();
            panic!("{}\n{diagnostics}", format_args!($($message)+));
        }
    };
}
pub(super) use check;

/// The key that opens the palette on this OS (the shipped keymap's).
pub(super) const OPEN_PALETTE: &str = if cfg!(target_os = "macos") {
    "cmd-shift-p"
} else {
    "ctrl-shift-p"
};

fn kind(group: &str, name: &str, plural: &str, short: &[&str]) -> ResourceKind {
    ResourceKind {
        gvk: Gvk::new(group, "v1", name),
        preferred: true,
        plural: plural.into(),
        singular: name.to_lowercase(),
        short_names: short.iter().map(|s| (*s).to_owned()).collect(),
        categories: Vec::new(),
        verbs: VerbSet::from_names(["get", "list", "watch", "delete"]),
        namespaced: true,
    }
}

/// The kinds the fixture cluster serves: the stock ones the tests jump to and a custom resource
/// with a short name of its own.
fn fixture_kinds() -> Vec<ResourceKind> {
    vec![
        kind("", "Pod", "pods", &["po"]),
        kind("", "ConfigMap", "configmaps", &["cm"]),
        kind("apps", "Deployment", "deployments", &["deploy"]),
        kind("example.com", "Widget", "widgets", &["wd"]),
    ]
}

/// The pods the fixture cluster holds, by namespace.
pub(super) const DEFAULT_PODS: [&str; 3] = ["nginx-a", "nginx-b", "web-1"];
/// The pods of `kube-system`.
pub(super) const SYSTEM_PODS: [&str; 2] = ["coredns-1", "kube-proxy-1"];

fn namespace(name: &str) -> Resource {
    Resource::from_json(serde_json::json!({
        "apiVersion": "v1", "kind": "Namespace", "metadata": { "name": name },
    }))
    .expect("namespace json")
}

/// Writes `settings.json` the way a user edit lands (the store notifies its observers).
fn set_user_settings(cx: &mut gpui::App, json: &str) {
    cx.update_global::<SettingsStore, _>(|store, _| {
        store.set_user_settings(json).expect("valid settings");
    });
}

impl App {
    /// The app over the fixture cluster, connected, with its Pods table open, focused and no row
    /// under the cursor yet. `vim` turns on `base_keymap: "vim"` the way `settings.json` does.
    pub(super) fn keyboard(cx: &mut TestAppContext, vim: bool) -> Self {
        let mut app = App::start_with(cx, TestPorts::seeded(), |cx| {
            if vim {
                set_user_settings(cx, r#"{ "base_keymap": "vim" }"#);
            }
        });
        app.serve(fixture_kinds());
        let resources = app
            .ports
            .connector
            .ports_for(&TestPorts::cluster_id())
            .resources;
        for name in ["default", "kube-system", "monitoring"] {
            resources.insert(namespace(name));
        }
        for name in DEFAULT_PODS {
            let builder = pod().namespace("default").name(name).running();
            let builder = if name.starts_with("nginx") {
                builder.label("app", "nginx")
            } else {
                builder.label("app", "web")
            };
            resources.insert(builder.build());
        }
        for name in SYSTEM_PODS {
            resources.insert(pod().namespace("kube-system").name(name).running().build());
        }
        for (ns, name) in [("default", "api"), ("kube-system", "coredns")] {
            resources.insert(deployment().namespace(ns).name(name).build());
        }
        // What the alias follower does when a session connects (it needs the Tokio bridge, which
        // the deterministic test runtime does not run): the cluster's discovered kinds become
        // aliases, so a custom resource has its short name.
        app.vcx.update(|_, cx| {
            AppState::global(cx)
                .services()
                .aliases
                .table(&TestPorts::cluster_id())
                .set_discovered(&fixture_kinds());
        });
        app.press("enter");
        app.tick();
        app.click("sidebar-entry-workloads/pods");
        app.tick();
        app.focus_table();
        app
    }

    /// Gives the keys to the table the cluster tab shows.
    pub(super) fn focus_table(&mut self) {
        let table = self.shown_table().expect("the cluster tab shows a table");
        self.vcx.update(|window, cx| {
            let focus = gpui::Focusable::focus_handle(table.read(cx), cx);
            window.focus(&focus, cx);
        });
        self.vcx.run_until_parked();
    }

    /// The table the cluster tab shows, when it shows one.
    pub(super) fn shown_table(&mut self) -> Option<Entity<ResourceTable>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            let active = ws.read(cx).active_item(cx)?.item_id();
            ws.read(cx)
                .items_of_type::<ResourceTable>()
                .into_iter()
                .find(|table| table.entity_id() == active)
        })
    }

    /// The kind and the row names of the shown table.
    pub(super) fn shown(&mut self) -> Option<(String, Vec<String>)> {
        let table = self.shown_table()?;
        Some(self.vcx.update(|_, cx| {
            let table = table.read(cx);
            let rows = table.read_rows(cx, |d| {
                d.rows().iter().map(|r| r.name().to_owned()).collect()
            });
            (table.gvk().kind.to_string(), rows)
        }))
    }

    /// The row names of the shown table, sorted.
    pub(super) fn shown_rows(&mut self) -> Vec<String> {
        let mut rows = self.shown().map(|(_, rows)| rows).unwrap_or_default();
        rows.sort();
        rows
    }

    /// The object under the cursor of the shown table.
    pub(super) fn cursor(&mut self) -> Option<ResourceRef> {
        let table = self.shown_table()?;
        self.vcx
            .update(|_, cx| table.read(cx).selected_refs(cx).into_iter().next())
    }

    /// The name of the object under the cursor of the shown table.
    pub(super) fn cursor_name(&mut self) -> Option<String> {
        self.cursor().map(|object| object.name.to_string())
    }

    pub(super) fn palette_open(&mut self) -> Option<Entity<CommandPalette>> {
        let ws = self.workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<CommandPalette>()
        })
    }

    pub(super) fn jump_bar_open(&mut self) -> Option<Entity<JumpBar>> {
        let ws = self.workspace();
        self.vcx
            .update(|_, cx| ws.read(cx).modal_layer().read(cx).active_modal::<JumpBar>())
    }

    pub(super) fn delete_dialog_open(&mut self) -> Option<Entity<DeleteDialog>> {
        let ws = self.tab_workspace();
        self.vcx.update(|_, cx| {
            ws.read(cx)
                .modal_layer()
                .read(cx)
                .active_modal::<DeleteDialog>()
        })
    }

    /// Types `text` into the focused field (no keymap), then lets the app settle.
    pub(super) fn type_text(&mut self, text: &str) {
        self.vcx.simulate_input(text);
        self.tick();
    }

    /// Where the keys go, for a failing assertion: the focused view (the innermost key context),
    /// the whole key-context stack, the modal on top and the table's cursor.
    pub(super) fn diagnostics(&mut self) -> String {
        let stack: Vec<String> = self
            .vcx
            .update(|window, _| window.context_stack())
            .iter()
            .map(|context| format!("{context:?}"))
            .collect();
        let modal = if self.palette_open().is_some() {
            "command palette"
        } else if self.jump_bar_open().is_some() {
            "jump bar"
        } else if self.delete_dialog_open().is_some() {
            "delete dialog"
        } else {
            "none"
        };
        let cursor = self.cursor_name();
        let shown = self.shown();
        format!(
            "  focused view: {}\n  key context stack: {}\n  modal: {modal}\n  shown table: {shown:?}\n  cursor: {cursor:?}",
            stack.last().map_or("(nothing focused)", String::as_str),
            stack.join(" > "),
        )
    }
}
