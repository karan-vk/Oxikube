//! The config editor's whole-file save of `keymap.json`: written while the file holds the text
//! the buffer started from, refused when another writer changed it.

use crate::edit::{UserKeymapSave, save_user_keymap_text};

#[test]
fn a_save_writes_while_the_file_is_as_it_was_read_and_refuses_a_stale_buffer() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("keymap.json");
    std::fs::write(&path, "[]").expect("write");
    // As read: written whole, no temporary left behind.
    let first = r#"[{ "bindings": { "cmd-k": "a::One" } }]"#;
    assert_eq!(
        save_user_keymap_text(&path, "[]", first).expect("saved"),
        UserKeymapSave::Saved
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), first);

    // The keymap editor wrote a binding since: the stale buffer is not renamed over it.
    let recorded = r#"[{ "bindings": { "cmd-k": "a::One", "cmd-j": "b::Two" } }]"#;
    std::fs::write(&path, recorded).expect("write");
    let stale = r#"[{ "bindings": { "cmd-k": "a::One" } } // a comment fixed
]"#;
    assert_eq!(
        save_user_keymap_text(&path, first, stale).expect("checked"),
        UserKeymapSave::ChangedOnDisk(recorded.to_owned())
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), recorded);

    // Saving again from what is on disk now overwrites knowingly.
    assert_eq!(
        save_user_keymap_text(&path, recorded, stale).expect("saved"),
        UserKeymapSave::Saved
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), stale);
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .expect("list")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect();
    assert_eq!(leftovers, ["keymap.json"], "no temporary file left");
}

#[test]
fn a_missing_file_is_empty_and_a_save_of_the_same_text_is_not_a_conflict() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("keymap.json");
    assert_eq!(
        save_user_keymap_text(&path, "", "[]").expect("saved"),
        UserKeymapSave::Saved
    );
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "[]");
    // Whatever the buffer started from, the file already holds this text: nothing to refuse.
    assert_eq!(
        save_user_keymap_text(&path, "older", "[]").expect("saved"),
        UserKeymapSave::Saved
    );
}
