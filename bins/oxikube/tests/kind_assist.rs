//! Hover docs and completion from the cluster's own schemas, through the app (E10-S05): a
//! manifest editor opened in the connected cluster's tab, the real `/openapi/v3` fetched through
//! its `SchemaPort`, and what a user gets from it.
//!
//! * a Pod: `ima` typed in a container completes `image` and `imagePullPolicy`, Enter inserts
//!   the field; `restartPolicy: ` completes the server's enum; the field's description (the
//!   server's own text) appears for the key under the cursor;
//! * the sample CRD (`widgets.test.oxikube.dev`, installed by `cargo xtask kind-up`): its `spec`
//!   offers the CRD's fields, the required ones first.
//!
//! `cargo test -p oxikube --features integration --test kind_assist` with `OXIKUBE_TEST_CONTEXT`
//! set (`cargo xtask kind-up`); without it the test returns at once. It only reads the cluster's
//! schemas: nothing is created and nothing is applied.
#![cfg(feature = "integration")]

mod kind_common;

use std::time::Duration;

use futures::executor::block_on;
use gpui::{Entity, TestAppContext, VisualTestContext};
use oxikube::app_state::AppState;
use oxikube_app::command_bus::DispatchContext;
use oxikube_domain::audit::Initiator;
use oxikube_domain::command::Command;
use oxikube_domain::ids::Gvk;
use oxikube_editor::view::ManifestEditor;
use oxikube_testkit::integration::{ensure_kind_context, test_context};
use oxikube_ui::editor::{CodeEditor, EditorApi as _};

use kind_common::{Launched, wait};

/// Lets the app run and the debounce timers (virtual time) pass.
fn settle(vcx: &mut VisualTestContext) {
    vcx.executor().advance_clock(Duration::from_millis(60));
    vcx.run_until_parked();
}

struct Editor {
    manifest: Entity<ManifestEditor>,
    code: Entity<CodeEditor>,
}

impl Editor {
    fn text(&self, vcx: &mut VisualTestContext) -> String {
        vcx.update(|window, cx| {
            self.code
                .update(cx, |code, cx| code.api(window, cx).text().to_string())
        })
    }

    fn menu(&self, vcx: &mut VisualTestContext) -> Vec<String> {
        vcx.update(|_, cx| self.code.read(cx).open_completions(cx))
    }

    fn select(&self, vcx: &mut VisualTestContext, at: usize) {
        vcx.update(|window, cx| {
            self.code
                .update(cx, |code, cx| code.api(window, cx).select(at..at))
        });
    }

    /// Waits until the editor holds the schema of `gvk` (fetched through the cluster's port).
    fn wait_for_schema(&self, vcx: &mut VisualTestContext, gvk: &Gvk) {
        wait(vcx, &format!("the schema of {}", gvk.kind), |vcx| {
            settle(vcx);
            vcx.update(|_, cx| {
                self.manifest
                    .read(cx)
                    .model()
                    .known()
                    .get(gvk)
                    .is_some_and(Option::is_some)
            })
        });
    }
}

#[gpui::test]
fn the_clusters_schemas_complete_and_document_a_pod_and_a_custom_resource(cx: &mut TestAppContext) {
    let Some(context) = test_context() else {
        return;
    };
    ensure_kind_context(&context).expect("a kind context");
    cx.executor().allow_parking();
    let Launched {
        mut vcx,
        tab,
        cluster,
        ..
    } = kind_common::launch(cx, &context);
    let workspace = vcx.update(|_, cx| tab.read(cx).workspace().clone());
    // `editor::NewManifest` for the connected cluster, as its keys and the palette send it.
    let state = vcx.update(|_, cx| AppState::global(cx));
    let bus = state.command_bus().expect("the bus").clone();
    block_on(bus.dispatch(
        Command::EditorNewManifest {
            cluster: Some(cluster),
        },
        DispatchContext::new(Initiator::Ui, "me"),
    ))
    .expect("the editor opens");
    vcx.run_until_parked();
    let manifest = vcx
        .update(|_, cx| workspace.read(cx).items_of_type::<ManifestEditor>())
        .pop()
        .expect("the editor opened in the cluster's tab");
    let code = vcx.update(|_, cx| manifest.read(cx).editor().clone());
    let editor = Editor { manifest, code };

    // --- a Pod -----------------------------------------------------------------------------
    vcx.simulate_input("apiVersion: v1\nkind: Pod\n");
    editor.wait_for_schema(&mut vcx, &Gvk::new("", "v1", "Pod"));

    vcx.simulate_input("spec:\n  restartPolicy: ");
    vcx.simulate_keystrokes("ctrl-space");
    wait(&mut vcx, "the enum of restartPolicy", |vcx| {
        settle(vcx);
        editor.menu(vcx) == ["Always", "Never", "OnFailure"]
    });
    vcx.simulate_keystrokes("escape");

    vcx.simulate_input("OnFailure\n  containers:\n    - name: app\n      ima");
    wait(&mut vcx, "image and imagePullPolicy", |vcx| {
        settle(vcx);
        editor.menu(vcx) == ["image", "imagePullPolicy"]
    });
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let text = editor.text(&mut vcx);
    assert!(text.ends_with("      image: "), "{text:?}");

    // The server's own description of the field under the cursor.
    editor.select(&mut vcx, text.find("restartPolicy").unwrap() + 3);
    vcx.simulate_keystrokes("ctrl-shift-space");
    let docs = kind_common::wait_value(&mut vcx, "the docs of restartPolicy", |vcx| {
        settle(vcx);
        vcx.update(|_, cx| editor.code.read(cx).open_hover(cx))
    })
    .1;
    assert!(
        docs.starts_with("**restartPolicy** · `string (enum)`"),
        "{docs}"
    );
    assert!(
        docs.contains("Restart policy for all containers within the pod."),
        "{docs}"
    );
    assert!(docs.contains("`spec.restartPolicy`"), "{docs}");
    vcx.simulate_keystrokes("escape");

    // --- the sample CRD ---------------------------------------------------------------------
    let end = editor.text(&mut vcx).len();
    editor.select(&mut vcx, end);
    vcx.simulate_input("\n---\napiVersion: test.oxikube.dev/v1\nkind: Widget\n");
    editor.wait_for_schema(&mut vcx, &Gvk::new("test.oxikube.dev", "v1", "Widget"));
    vcx.simulate_input("spec:\n  ");
    vcx.simulate_keystrokes("ctrl-space");
    wait(&mut vcx, "the CRD's fields, required first", |vcx| {
        settle(vcx);
        editor.menu(vcx) == ["replicas", "size", "owner"]
    });
}
