use super::*;
use std::path::PathBuf;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icon_themes")
}

fn theme_json(file_icons: &str) -> String {
    format!(
        r#"{{ "name": "T", "themes": [{{ "name": "T", "appearance": "light", "file_icons": {{ {file_icons} }} }}] }}"#
    )
}

#[test]
fn reads_a_zed_icon_theme_with_stems_suffixes_and_chevrons() {
    let root = fixtures();
    let source = read_icon_theme_file(&root.join("zed-sample.json"), &root).unwrap();
    assert!(source.problems.is_empty(), "{:?}", source.problems);
    let family = &source.family;
    assert_eq!(family.name, "Zed Sample Icons");
    assert_eq!(family.author, "Oxikube tests");
    assert!(
        family
            .schema
            .as_deref()
            .is_some_and(|s| s.contains("icon_themes"))
    );
    let theme = &family.themes[0];
    assert_eq!(theme.file_stems["Dockerfile"], "docker");
    assert_eq!(theme.file_suffixes["rs"], "rust");
    assert_eq!(theme.file_suffixes["yml"], "yaml");
    assert_eq!(
        theme.chevron_icons.collapsed.as_deref(),
        Some("zed-sample/icons/chevron-right.svg"),
        "paths are normalised"
    );
    assert_eq!(
        theme.chevron_icons.expanded.as_deref(),
        Some("zed-sample/icons/chevron-down.svg")
    );
    assert_eq!(
        theme.directory_icons.expanded.as_deref(),
        Some("zed-sample/icons/folder-open.svg")
    );
    assert!(theme.named_directory_icons.contains_key(".git"));
    assert_eq!(theme.file_icons["rust"].path, "zed-sample/icons/rust.svg");
    // Each distinct file is read once (git.svg is named twice).
    assert_eq!(source.files.len(), 8);
    assert!(source.files["zed-sample/icons/rust.svg"].starts_with(b"<svg"));
}

#[test]
fn a_path_with_parent_components_is_refused_and_nothing_outside_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("icon_themes");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(dir.path().join("secret.svg"), "<svg>secret</svg>").unwrap();
    std::fs::write(root.join("ok.svg"), "<svg>ok</svg>").unwrap();

    let json = theme_json(
        r#""kind:Pod": { "path": "../secret.svg" }, "kind:Node": { "path": "./ok.svg" }"#,
    );
    let source = read_icon_theme(&json, &root).unwrap();
    let icons = &source.family.themes[0].file_icons;
    assert!(
        !icons.contains_key("kind:Pod"),
        "the escaping icon is dropped"
    );
    assert_eq!(icons["kind:Node"].path, "ok.svg");
    assert_eq!(source.files.len(), 1);
    assert!(
        source
            .files
            .values()
            .all(|bytes| !bytes.starts_with(b"<svg>secret"))
    );
    assert!(
        source.problems.iter().any(|p| p.contains("leaves")),
        "{:?}",
        source.problems
    );
}

#[test]
fn absolute_and_missing_paths_are_dropped_with_a_problem() {
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("secret.svg");
    std::fs::write(&secret, "<svg/>").unwrap();
    let json = theme_json(&format!(
        r#""a": {{ "path": {:?} }}, "b": {{ "path": "./missing.svg" }}, "c": {{ "path": "x.png" }}"#,
        secret.display().to_string()
    ));
    let source = read_icon_theme(&json, dir.path()).unwrap();
    assert!(source.family.themes[0].file_icons.is_empty());
    assert!(source.files.is_empty());
    assert_eq!(source.problems.len(), 3, "{:?}", source.problems);
    assert!(
        source.problems[0].contains("absolute"),
        "{:?}",
        source.problems
    );
}

#[cfg(unix)]
#[test]
fn a_symlink_out_of_the_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("icon_themes");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(dir.path().join("secret.svg"), "<svg>secret</svg>").unwrap();
    std::os::unix::fs::symlink(dir.path().join("secret.svg"), root.join("link.svg")).unwrap();

    let source = read_icon_theme(&theme_json(r#""a": { "path": "link.svg" }"#), &root).unwrap();
    assert!(source.files.is_empty());
    assert!(
        source.problems[0].contains("resolves outside"),
        "{:?}",
        source.problems
    );
}

#[test]
fn files_that_are_not_icon_themes_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        read_icon_theme("{ nope", dir.path()),
        Err(IconThemeError::Json(_))
    ));
    assert_eq!(
        read_icon_theme(r#"{ "name": "Empty", "themes": [] }"#, dir.path()),
        Err(IconThemeError::NoThemes)
    );
    assert!(matches!(
        read_icon_theme_file(&dir.path().join("missing.json"), dir.path()),
        Err(IconThemeError::Read(_))
    ));
}
