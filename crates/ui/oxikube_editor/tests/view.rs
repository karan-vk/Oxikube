//! The manifest editor in a workspace (E10-S04): opened by `editor::NewManifest` through the
//! shipped keymap and the window's `EditorViews`, typed into, validated after the debounce
//! against the cluster's schemas (a fake `SchemaPort` on the deterministic runtime), toggled
//! through the keys and through its own toolbar (split beside another editor), and its key
//! context.

#[path = "view_common/mod.rs"]
mod common;

use common::*;
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, Render, TestAppContext, VisualTestContext, Window, div,
};
use oxikube_domain::command::{Command, ViewContext};
use oxikube_editor::view::{ManifestEditor, VALIDATION_DEBOUNCE};
use oxikube_workspace::command_surface::view_of_focus;
use oxikube_workspace::{Item, ItemEvent, SplitDirection, TabContent};

#[gpui::test]
fn the_new_manifest_key_opens_a_focused_editor(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    assert_eq!(
        f.sent.borrow().as_slice(),
        [Command::EditorNewManifest { cluster: None }]
    );
    let title = f.vcx.read(|cx| editor.read(cx).tab_content(cx).title);
    assert_eq!(title.as_ref(), "Untitled-1");
    f.type_text("kind: Pod\n");
    assert_eq!(f.text(&editor), "kind: Pod\n");
    let dirty = f.vcx.read(|cx| editor.read(cx).tab_content(cx).dirty);
    assert!(dirty, "an edited buffer shows the dirty dot");
    // Never saved with the layout: a manifest can hold Secret data.
    assert!(<ManifestEditor as Item>::serialized_kind().is_none());
}

const BROKEN: &str = "apiVersion: v1\nkind: ConfigMap\ndata:\n  a: 1\n";

#[gpui::test]
fn typing_is_validated_after_the_debounce_against_the_cluster_schema(cx: &mut TestAppContext) {
    let mut f = setup(cx, true);
    let editor = f.open_editor();
    f.type_text(BROKEN);
    f.vcx.run_until_parked();
    assert!(f.squiggles(&editor).is_empty(), "nothing before the pause");

    f.advance(VALIDATION_DEBOUNCE);
    // The schema was fetched off the UI thread and the buffer validated again.
    let value = BROKEN.rfind('1').unwrap();
    assert_eq!(f.squiggles(&editor), vec![value..value + 1]);
    let problems = f.vcx.read(|cx| editor.read(cx).model().problems());
    assert_eq!((problems.errors, problems.warnings), (1, 0));
    assert_eq!(f.port.recorded_calls().len(), 1, "fetched once");

    // A new keystroke drops the squiggles until the next pause; a pause shorter than the
    // debounce, followed by more typing, validates nothing.
    f.type_text("#");
    assert!(f.squiggles(&editor).is_empty());
    f.advance(VALIDATION_DEBOUNCE / 2);
    f.type_text(" x");
    f.advance(VALIDATION_DEBOUNCE / 2);
    assert!(f.squiggles(&editor).is_empty(), "the timer restarted");
    f.advance(VALIDATION_DEBOUNCE);
    assert_eq!(f.squiggles(&editor), vec![value..value + 1]);
    assert_eq!(f.port.recorded_calls().len(), 1, "the schema is kept");
}

#[gpui::test]
fn the_keys_toggle_soft_wrap_and_read_only_through_the_bus(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let editor = f.open_editor();
    f.vcx.simulate_keystrokes("alt-z");
    f.vcx.run_until_parked();
    let wrap = f
        .vcx
        .read(|cx| editor.read(cx).editor().read(cx).soft_wrap());
    assert!(wrap);
    assert!(f.sent.borrow().contains(&Command::EditorToggleSoftWrap));

    f.vcx.simulate_keystrokes(read_only_key());
    f.vcx.run_until_parked();
    assert!(f.sent.borrow().contains(&Command::EditorToggleReadOnly));
    f.type_text("zz");
    assert_eq!(f.text(&editor), "", "a read-only buffer refuses typing");
    f.vcx.simulate_keystrokes(read_only_key());
    f.vcx.run_until_parked();
    f.type_text("a");
    assert_eq!(f.text(&editor), "a");
}

impl Fixture {
    fn focus(&mut self, editor: &Entity<ManifestEditor>) {
        let focus = self.vcx.read(|cx| editor.read(cx).focus_handle(cx));
        self.vcx.update(|window, cx| window.focus(&focus, cx));
        self.vcx.run_until_parked();
    }

    /// Clicks the toolbar button tagged `selector` (`<id>-<editor title>`).
    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .vcx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
        self.vcx
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        self.vcx.run_until_parked();
    }

    fn soft_wrap(&mut self, editor: &Entity<ManifestEditor>) -> bool {
        self.vcx
            .read(|cx| editor.read(cx).editor().read(cx).soft_wrap())
    }

    fn read_only(&mut self, editor: &Entity<ManifestEditor>) -> bool {
        self.vcx
            .read(|cx| editor.read(cx).editor().read(cx).is_read_only())
    }
}

/// The toolbar's buttons keep the focus where it was on mouse-down, and the bus commands carry
/// no target: the click must still act on the editor whose toolbar was clicked, not on the one
/// being typed in.
#[gpui::test]
fn a_toolbar_toggle_acts_on_its_own_editor_not_the_focused_one(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let left = f.open_editor();
    let right = f.open_editor();
    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.split_active_pane(SplitDirection::Right, window, cx)
                .expect("Untitled-2 moves to a pane on the right");
        });
    });
    f.vcx.run_until_parked();

    // Typing in the left editor, then clicking Wrap on the right one's toolbar.
    f.focus(&left);
    f.type_text("a");
    f.click("manifest-wrap-Untitled-2");
    assert!(f.sent.borrow().contains(&Command::EditorToggleSoftWrap));
    assert!(f.soft_wrap(&right), "the clicked editor wraps");
    assert!(!f.soft_wrap(&left), "the editor typed in is untouched");

    // The same for Read-only: the editor typed in stays editable.
    f.focus(&left);
    f.click("manifest-read-only-Untitled-2");
    assert!(f.read_only(&right), "the clicked editor is read-only");
    assert!(!f.read_only(&left), "the editor typed in is untouched");
    f.focus(&left);
    f.type_text("b");
    assert_eq!(f.text(&left), "ab");
    f.focus(&right);
    f.type_text("z");
    assert_eq!(f.text(&right), "", "the clicked editor refuses typing");

    // And back on the left editor's own toolbar.
    f.focus(&right);
    f.click("manifest-wrap-Untitled-1");
    assert!(f.soft_wrap(&left));
    assert!(f.soft_wrap(&right), "the other editor keeps its wrap");
}

/// Stands for the palette: a focusable item with the `Palette` key context.
struct PaletteProbe {
    focus: FocusHandle,
}

impl EventEmitter<ItemEvent> for PaletteProbe {}

impl Focusable for PaletteProbe {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for PaletteProbe {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().key_context("Palette").track_focus(&self.focus)
    }
}

impl Item for PaletteProbe {
    fn tab_content(&self, _: &App) -> TabContent {
        TabContent::new("Palette")
    }
}

fn focused_contexts(vcx: &mut VisualTestContext) -> (Vec<String>, ViewContext) {
    vcx.update(|window, _| {
        let stack = window
            .context_stack()
            .iter()
            .filter_map(|context| context.primary().map(|entry| entry.key.to_string()))
            .collect();
        (stack, view_of_focus(window))
    })
}

#[gpui::test]
fn the_manifest_editor_context_is_active_only_while_it_has_focus(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let _editor = f.open_editor();
    let editing = f.vcx.update(|window, _| {
        window
            .context_stack()
            .iter()
            .any(|context| context.contains("ManifestEditor") && context.contains("Editing"))
    });
    assert!(
        editing,
        "the buffer has the focus: ManifestEditor and Editing"
    );
    let (stack, view) = focused_contexts(&mut f.vcx);
    assert!(stack.iter().any(|c| c == "ManifestEditor"), "{stack:?}");
    assert_eq!(view, ViewContext::Editor);

    let workspace = f.workspace.clone();
    f.vcx.update(|window, cx| {
        let probe = cx.new(|cx| PaletteProbe {
            focus: cx.focus_handle(),
        });
        workspace.update(cx, |ws, cx| ws.open_item(probe, window, cx));
    });
    f.vcx.run_until_parked();
    let (stack, view) = focused_contexts(&mut f.vcx);
    assert!(stack.iter().any(|c| c == "Palette"), "{stack:?}");
    assert!(!stack.iter().any(|c| c == "ManifestEditor"), "{stack:?}");
    assert_ne!(view, ViewContext::Editor);
}
