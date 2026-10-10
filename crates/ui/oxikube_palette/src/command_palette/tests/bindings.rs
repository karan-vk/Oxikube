//! The key binding on a row: read from the live keymap (AC1: "lists commands with bindings").

use gpui::{TestAppContext, px};
use oxikube_domain::command::CommandId;
use oxikube_keymap::KeymapOptions;

use super::{Fixture, OPEN_KEY, SHOW_ALL_KEY};
use crate::command_palette::render::binding_strokes;

/// The width the key caps of the row listing `id` take, `None` when the row is not drawn.
fn caps_width(f: &mut Fixture, id: CommandId) -> Option<f32> {
    let ix = f.listed().iter().position(|listed| *listed == id)?;
    f.settle();
    // `debug_bounds` wants a `'static` selector: leaking a few bytes in a test is fine.
    let selector: &'static str = Box::leak(format!("palette-keys-{ix}").into_boxed_str());
    let bounds = f.vcx.debug_bounds(selector)?;
    Some(bounds.size.width / px(1.))
}

/// Types `query` and checks the command `id` is the selected (first) row, then measures its caps.
fn find(f: &mut Fixture, query: &str, id: CommandId) -> Option<f32> {
    f.type_text(query);
    assert_eq!(f.selected(), Some(id));
    caps_width(f, id)
}

#[gpui::test]
fn a_bound_command_is_drawn_with_its_key_caps(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    let width = find(&mut f, "zoom in", CommandId::VIEW_ZOOM_IN).expect("the row is drawn");
    assert!(width > 0., "view::ZoomIn is bound: its caps take room");
}

#[gpui::test]
fn an_unbound_command_has_no_key_caps(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    f.open();
    // `Scale Workload` has no default key.
    let width = find(&mut f, "scale workload", CommandId::WORKLOAD_SCALE).expect("drawn");
    assert_eq!(width, 0., "no binding, no caps");
}

#[gpui::test]
fn the_palette_row_shows_the_keys_the_keymap_gives_the_palette_itself(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &[]);
    f.open();
    let strokes = f.vcx.update(|_, cx| binding_strokes("palette::Toggle", cx));
    assert_eq!(strokes, [OPEN_KEY], "the shipped binding that opened it");
    let width = find(&mut f, "toggle command palette", CommandId::PALETTE_TOGGLE).expect("drawn");
    assert!(width > 0.);
    // The footer's "Show all" carries its own binding.
    let shown = f
        .vcx
        .update(|_, cx| binding_strokes("palette::ToggleShowAll", cx));
    assert_eq!(shown, [SHOW_ALL_KEY]);
    let footer = f.vcx.debug_bounds("palette-show-all-keys").expect("drawn");
    assert!(footer.size.width > px(0.));
}

#[gpui::test]
fn a_keymap_override_changes_the_hint_on_the_next_frame(cx: &mut TestAppContext) {
    let mut f = Fixture::declared(cx, &["web"]);
    let before = f.vcx.update(|_, cx| binding_strokes("view::ZoomIn", cx));
    assert!(!before.is_empty(), "shipped: view::ZoomIn is bound");

    // `keymap.json` rebinds Zoom In to a chord of its own, and the palette shows it.
    f.vcx.update(|_, cx| {
        oxikube_keymap::init_with_text(
            r#"[{"bindings": {"ctrl-alt-9": "view::ZoomIn"}}]"#,
            KeymapOptions::default(),
            cx,
        );
    });
    let after = f.vcx.update(|_, cx| binding_strokes("view::ZoomIn", cx));
    assert!(
        after == ["ctrl-alt-9"],
        "the override is read from the live keymap: {before:?} -> {after:?}"
    );

    // And unbinding every key of a command removes its caps.
    f.vcx.update(|_, cx| {
        oxikube_keymap::init_with_text(
            r#"[{"bindings": {"cmd-=": null, "cmd-+": null, "ctrl-=": null, "ctrl-+": null}}]"#,
            KeymapOptions::default(),
            cx,
        );
    });
    assert!(
        f.vcx
            .update(|_, cx| binding_strokes("view::ZoomIn", cx))
            .is_empty()
    );
    f.open();
    let width = find(&mut f, "zoom in", CommandId::VIEW_ZOOM_IN).expect("drawn");
    assert_eq!(width, 0., "an unbound command shows no caps");
}
