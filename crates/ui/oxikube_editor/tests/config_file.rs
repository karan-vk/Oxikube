//! `keymap.json` in the editor (E21-S03), end to end in a workspace: the open request reads the
//! user's file into an editor, the keymap validator's findings reach the squiggles (and replace a
//! schema pass), a result for an older buffer is dropped, and Save writes the path instead of
//! starting a Kubernetes apply.

#[path = "view_common/mod.rs"]
mod common;

use common::*;
use gpui::TestAppContext;
use oxikube_editor::view::{
    ConfigFile, ConfigFileKind, EditorRequest, ManifestEditor, NoticeLevel, OpenConfigFile,
    VALIDATION_DEBOUNCE,
};
use oxikube_ui::editor::EditorApi as _;

const BAD_CONTEXT: &str = "[{\"context\": \"ResourceTabel\", \"bindings\": {}}]";

/// A window whose keymap was installed from `dir`, and the editor open on its `keymap.json`
/// holding `text`.
fn open_keymap(f: &mut Fixture, dir: &std::path::Path, text: &str) -> gpui::Entity<ManifestEditor> {
    std::fs::write(dir.join("keymap.json"), text).expect("write the keymap");
    f.vcx.update(|_, cx| {
        oxikube_keymap::init_with_dir(dir, oxikube_keymap::KeymapOptions::default(), cx);
    });
    let path = f
        .vcx
        .read(oxikube_keymap::user_keymap_file)
        .expect("installed from a directory");
    let file = ConfigFile {
        kind: ConfigFileKind::Keymap,
        path,
    };
    f.sink
        .send(EditorRequest::OpenConfigFile(OpenConfigFile { file }))
        .expect("the window is open");
    f.vcx.run_until_parked();
    f.advance(VALIDATION_DEBOUNCE);
    let editors = f
        .vcx
        .read(|cx| f.workspace.read(cx).items_of_type::<ManifestEditor>());
    assert_eq!(editors.len(), 1, "the file opened in one editor");
    editors.into_iter().next().expect("one editor")
}

#[gpui::test]
fn the_path_is_recognised_as_the_keymap(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dir = tempfile::tempdir().expect("a temp dir");
    f.vcx.update(|_, cx| {
        oxikube_keymap::init_with_dir(dir.path(), oxikube_keymap::KeymapOptions::default(), cx);
    });
    let (own, other) = f.vcx.read(|cx| {
        let own = oxikube_keymap::user_keymap_file(cx).expect("a path");
        let other = dir.path().join("settings.json");
        (
            ConfigFileKind::of_path(&own, cx),
            ConfigFileKind::of_path(&other, cx),
        )
    });
    assert_eq!(own, Some(ConfigFileKind::Keymap));
    assert_eq!(other, None);
}

#[gpui::test]
fn opening_the_keymap_underlines_its_problems(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dir = tempfile::tempdir().expect("a temp dir");
    let editor = open_keymap(&mut f, dir.path(), BAD_CONTEXT);
    assert_eq!(f.text(&editor), BAD_CONTEXT);
    let squiggles = f.squiggles(&editor);
    assert_eq!(squiggles.len(), 1, "the unknown context: {squiggles:?}");
    assert_eq!(&BAD_CONTEXT[squiggles[0].clone()], "\"ResourceTabel\"");
    assert!(f.vcx.read(|cx| editor.read(cx).config_file().is_some()));
}

#[gpui::test]
fn editing_revalidates_after_the_debounce_and_clears_fixed_problems(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dir = tempfile::tempdir().expect("a temp dir");
    let editor = open_keymap(&mut f, dir.path(), BAD_CONTEXT);
    assert_eq!(f.squiggles(&editor).len(), 1);

    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| code.api(window, cx).replace_text("[]"));
    });
    // Once the debounce has elapsed the new buffer is re-checked and the old underline is gone.
    f.advance(VALIDATION_DEBOUNCE);
    assert!(f.squiggles(&editor).is_empty());
}

#[gpui::test]
fn save_writes_the_file_and_never_starts_a_cluster_apply(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dir = tempfile::tempdir().expect("a temp dir");
    let editor = open_keymap(&mut f, dir.path(), BAD_CONTEXT);
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| code.api(window, cx).replace_text("[]"));
    });
    f.advance(VALIDATION_DEBOUNCE);

    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.begin_apply(window, cx));
    });
    f.vcx.run_until_parked();
    // The write is on the background executor: let it finish.
    for _ in 0..50 {
        if std::fs::read_to_string(dir.path().join("keymap.json")).expect("the file") == "[]" {
            break;
        }
        f.vcx.executor().allow_parking();
        std::thread::sleep(std::time::Duration::from_millis(10));
        f.vcx.run_until_parked();
    }
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keymap.json")).expect("the file"),
        "[]"
    );
    let notice = f.vcx.read(|cx| {
        editor
            .read(cx)
            .notice()
            .map(|n| (n.level, n.text.to_string()))
    });
    let (level, text) = notice.expect("a notice");
    assert_eq!(level, NoticeLevel::Info, "{text}");
    assert_eq!(text, "Saved");
}

/// Run the save (`resource::Apply`) and wait for its background write to report (the notice
/// changes).
fn save(f: &mut Fixture, editor: &gpui::Entity<ManifestEditor>) -> (NoticeLevel, String) {
    let notice = |f: &mut Fixture| {
        f.vcx.read(|cx| {
            editor
                .read(cx)
                .notice()
                .map(|n| (n.level, n.text.to_string()))
        })
    };
    let before = notice(f);
    f.vcx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.begin_apply(window, cx));
    });
    for _ in 0..100 {
        f.vcx.run_until_parked();
        if let Some(now) = notice(f).filter(|now| Some(now) != before.as_ref()) {
            return now;
        }
        f.vcx.executor().allow_parking();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("the save never reported");
}

#[gpui::test]
fn a_save_never_overwrites_a_change_made_on_disk_since_the_file_opened(cx: &mut TestAppContext) {
    let mut f = setup(cx, false);
    let dir = tempfile::tempdir().expect("a temp dir");
    let editor = open_keymap(&mut f, dir.path(), "[]");
    // The keymap editor records a binding meanwhile (its write goes straight to the file).
    let recorded = r#"[{ "bindings": { "cmd-shift-p": "palette::Toggle" } }]"#;
    std::fs::write(dir.path().join("keymap.json"), recorded).expect("write");
    let code = f.vcx.read(|cx| editor.read(cx).editor().clone());
    f.vcx.update(|window, cx| {
        code.update(cx, |code, cx| {
            code.api(window, cx).replace_text("[] // fixed")
        });
    });
    f.advance(VALIDATION_DEBOUNCE);

    let (level, text) = save(&mut f, &editor);
    assert_eq!(level, NoticeLevel::Warning, "{text}");
    assert!(text.contains("changed on disk"), "{text}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keymap.json")).expect("the file"),
        recorded,
        "the recorded binding survives the stale buffer"
    );

    // Saving again, having been told, overwrites.
    let (level, text) = save(&mut f, &editor);
    assert_eq!((level, text.as_str()), (NoticeLevel::Info, "Saved"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keymap.json")).expect("the file"),
        "[] // fixed"
    );
}
