//! The `base_keymap` overlays (E21-S09): `default`, `k9s` and `vim` switched live, the layer order
//! (user over overlay over default, `null` across layers), and that nothing changes where text is
//! typed. Driven through `simulate_keystrokes`, so it is GPUI's real dispatch.

mod common;

use common::*;
use gpui::TestAppContext;
use oxikube_keymap::file::parse_keymap;
use oxikube_keymap::{
    BaseKeymap, KeymapLayer, KeymapOptions, KeymapPlatform, init_with_dir, init_with_text,
    set_base_keymap,
};

fn options(base_keymap: BaseKeymap) -> KeymapOptions {
    KeymapOptions {
        platform: KeymapPlatform::MacOs,
        base_keymap,
    }
}

fn install(cx: &mut TestAppContext, base: BaseKeymap, user: &str) {
    cx.update(|cx| init_with_text(user, options(base), cx));
    let problems = cx.read(oxikube_keymap::diagnostics);
    assert!(problems.is_empty(), "{problems:?}");
}

fn switch(cx: &mut TestAppContext, base: BaseKeymap) {
    cx.update(|cx| set_base_keymap(cx, base));
}

const ALL: [BaseKeymap; 3] = [BaseKeymap::Default, BaseKeymap::K9s, BaseKeymap::Vim];

/// Which of the overlays' marker keys act in a table: `g g` (vim) and `c` (k9s).
fn table_keys(cx: &mut TestAppContext) -> (bool, bool) {
    let (table, log) = probe(cx, "ResourceTable");
    (
        press(cx, table, &log, "g g") == ["SelectFirst"],
        press(cx, table, &log, "c") == ["CopyName"],
    )
}

#[gpui::test]
fn switching_the_base_keymap_re_layers_the_defaults_live(cx: &mut TestAppContext) {
    install(cx, BaseKeymap::Default, "");
    assert_eq!(table_keys(cx), (false, false));
    switch(cx, BaseKeymap::K9s);
    assert_eq!(table_keys(cx), (false, true), "k9s on");
    switch(cx, BaseKeymap::Vim);
    assert_eq!(table_keys(cx), (true, false), "vim replaces k9s");
    switch(cx, BaseKeymap::Default);
    assert_eq!(table_keys(cx), (false, false), "back to the defaults");
    switch(cx, BaseKeymap::Vim);
    switch(cx, BaseKeymap::K9s);
    assert_eq!(table_keys(cx), (false, true));
}

#[gpui::test]
fn k9s_closes_the_detail_drawer_with_q_and_the_defaults_do_not(cx: &mut TestAppContext) {
    install(cx, BaseKeymap::Default, "");
    let (drawer, log) = probe(cx, "DetailDrawer");
    assert!(press(cx, drawer, &log, "q").is_empty());
    switch(cx, BaseKeymap::K9s);
    assert_eq!(press(cx, drawer, &log, "q"), ["Close"]);
    let (editing, editing_log) = probe(cx, "DetailDrawer Editing");
    assert!(
        press(cx, editing, &editing_log, "q").is_empty(),
        "q is text in the find field"
    );
}

#[gpui::test]
fn text_entry_views_are_unaffected_by_every_base_keymap(cx: &mut TestAppContext) {
    for base in ALL {
        install(cx, base, "");
        for context in [
            "Terminal",
            "ManifestEditor",
            "ResourceTable Editing",
            "Input",
            "Palette",
            "JumpBar",
        ] {
            let (window, log) = probe(cx, context);
            for keys in ["c", "q", "g g", "shift-g", "j", "d d", "y y"] {
                assert!(
                    press(cx, window, &log, keys).is_empty(),
                    "{keys} in {context} under {base:?}"
                );
            }
        }
    }
}

#[gpui::test]
fn a_half_typed_sequence_cannot_complete_a_binding_of_the_old_layer(cx: &mut TestAppContext) {
    install(cx, BaseKeymap::Vim, "");
    let (table, log) = probe(cx, "ResourceTable");
    assert!(press(cx, table, &log, "g").is_empty(), "first half of g g");
    switch(cx, BaseKeymap::Default);
    assert!(
        press(cx, table, &log, "g").is_empty(),
        "g g is gone with the vim layer"
    );
    switch(cx, BaseKeymap::K9s);
    assert!(press(cx, table, &log, "g").is_empty());
    assert_eq!(press(cx, table, &log, "c"), ["CopyName"], "keys work again");
}

#[gpui::test]
fn the_user_layer_beats_the_overlay_and_null_unbinds_across_layers(cx: &mut TestAppContext) {
    let user = r#"[{"context": "ResourceTable", "bindings": {
        "c": "kmtest::Alpha",
        "shift-g": null
    }}]"#;
    install(cx, BaseKeymap::K9s, user);
    let (table, log) = probe(cx, "ResourceTable");
    assert_eq!(press(cx, table, &log, "c"), ["Alpha"], "user beats k9s");
    switch(cx, BaseKeymap::Vim);
    assert_eq!(press(cx, table, &log, "g g"), ["SelectFirst"]);
    assert!(
        press(cx, table, &log, "shift-g").is_empty(),
        "the user's null hides the vim binding"
    );
    switch(cx, BaseKeymap::Default);
    assert!(press(cx, table, &log, "shift-g").is_empty());
}

#[gpui::test]
fn the_overlay_beats_the_default_for_the_same_key(cx: &mut TestAppContext) {
    // The defaults (k9s) delete on ctrl-d; the vim layer scrolls there instead.
    install(cx, BaseKeymap::Default, "");
    let (table, log) = probe(cx, "ResourceTable");
    assert_eq!(press(cx, table, &log, "ctrl-d"), ["DeleteSelected"]);
    switch(cx, BaseKeymap::Vim);
    assert_eq!(press(cx, table, &log, "ctrl-d"), ["SelectHalfPageDown"]);
    switch(cx, BaseKeymap::K9s);
    assert_eq!(press(cx, table, &log, "ctrl-d"), ["DeleteSelected"]);
}

#[gpui::test]
fn switching_never_touches_the_users_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keymap.json");
    let text =
        "// mine\n[{\"context\": \"ResourceTable\", \"bindings\": {\"x\": \"kmtest::Beta\"}}]\n";
    std::fs::write(&path, text).unwrap();
    cx.update(|cx| init_with_dir(dir.path(), options(BaseKeymap::Default), cx));
    for base in [BaseKeymap::Vim, BaseKeymap::K9s, BaseKeymap::Default] {
        switch(cx, base);
        cx.run_until_parked();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text, "{base:?}");
    }
    let (table, log) = probe(cx, "ResourceTable");
    assert_eq!(
        press(cx, table, &log, "x"),
        ["Beta"],
        "the user file stays live"
    );
}

#[test]
fn each_overlay_is_scoped_to_contexts_without_text_entry() {
    let parsed = parse_keymap(oxikube_assets::k9s_keymap(), KeymapLayer::K9s).unwrap();
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    for (_, section) in parsed.sections {
        let context = section
            .context_expr()
            .expect("k9s bindings are never global");
        assert!(context.contains("!Editing"), "{context}");
    }
}

#[gpui::test]
fn the_setting_switches_default_vim_k9s_and_back_live(cx: &mut TestAppContext) {
    use gpui::UpdateGlobal as _;
    use oxikube_settings::SettingsStore;
    cx.update(|cx| {
        cx.set_global(SettingsStore::new(oxikube_assets::default_settings()).unwrap());
        init_with_text("", options(BaseKeymap::Default), cx);
    });
    let set_user = |cx: &mut TestAppContext, text: &str| {
        cx.update(|cx| {
            SettingsStore::update_global(cx, |store, _| {
                store.set_user_settings(text).expect("valid settings");
            });
        });
        cx.run_until_parked();
    };
    assert_eq!(table_keys(cx), (false, false));
    for (setting, expected) in [
        ("vim", (true, false)),
        ("k9s", (false, true)),
        ("default", (false, false)),
        ("k9s", (false, true)),
    ] {
        set_user(cx, &format!(r#"{{ "base_keymap": "{setting}" }}"#));
        assert_eq!(table_keys(cx), expected, "base_keymap: {setting}");
    }
}
