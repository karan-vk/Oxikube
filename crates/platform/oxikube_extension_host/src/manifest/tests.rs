use super::*;

const MINIMAL: &str = include_str!("../../tests/fixtures/minimal.toml");
const FULL: &str = include_str!("../../tests/fixtures/full.toml");

#[test]
fn minimal_manifest_parses() {
    let m = ExtensionManifest::parse(MINIMAL).expect("minimal parses");
    assert_eq!(m.id, "hello");
    assert_eq!(m.version, Version::new(0, 1, 0));
    assert!(m.commands.is_empty() && m.capabilities.is_empty() && m.lib.is_none());
}

#[test]
fn full_manifest_parses() {
    let m = ExtensionManifest::parse(FULL).expect("full parses");
    assert_eq!(m.lib.expect("lib").kind, LibKind::Rust);
    assert_eq!(m.themes, ["themes/dark.json"]);
    assert_eq!(m.commands.len(), 2);
    assert_eq!(
        m.commands[0].scopes,
        [CommandScope::Global, CommandScope::Cluster]
    );
    assert_eq!(
        m.commands[1].scopes,
        [CommandScope::Global],
        "scopes default to global"
    );
    assert_eq!(m.context_servers[0].id, "echo");
    let kinds: Vec<_> = m
        .capabilities
        .iter()
        .map(ExtensionCapability::kind)
        .collect();
    assert_eq!(kinds, ["process:exec", "download_file", "kube:read"]);
}

#[test]
fn unknown_top_level_field_is_named() {
    let err = ExtensionManifest::parse(&format!("{MINIMAL}colour = \"red\"\n")).unwrap_err();
    assert!(
        matches!(&err, ManifestError::UnknownField { field, .. } if field == "colour"),
        "{err}"
    );
    assert!(err.to_string().contains("colour"));
}

#[test]
fn unknown_field_inside_command_is_named() {
    let text = format!("{MINIMAL}[[commands]]\nid = \"a\"\ntitle = \"A\"\nicon = \"x\"\n");
    let err = ExtensionManifest::parse(&text).unwrap_err();
    assert!(
        matches!(&err, ManifestError::UnknownField { field, .. } if field == "icon"),
        "{err}"
    );
    assert!(err.to_string().contains("line 8"), "span in message: {err}");
}

#[test]
fn syntax_and_missing_field_errors() {
    assert!(matches!(
        ExtensionManifest::parse("id = "),
        Err(ManifestError::Syntax(_))
    ));
    let err =
        ExtensionManifest::parse("id = \"a\"\nname = \"A\"\nschema_version = 1\n").unwrap_err();
    assert!(
        matches!(&err, ManifestError::MissingField { field, .. } if field == "version"),
        "{err}"
    );
}

#[test]
fn capabilities_round_trip_each_variant() {
    let m = ExtensionManifest::parse(FULL).expect("full parses");
    assert_eq!(
        m.capabilities,
        [
            ExtensionCapability::ProcessExec {
                command: "kubectl".into(),
                args: vec!["get".into(), "*".into()]
            },
            ExtensionCapability::DownloadFile {
                host: "github.com".into(),
                path: "/example/*".into()
            },
            ExtensionCapability::KubeRead {
                contexts: vec!["kind-*".into()]
            },
        ]
    );
}

fn with(extra: &str) -> String {
    format!("{MINIMAL}{extra}")
}

#[test]
fn invalid_inputs_are_rejected() {
    let cases = [
        ("bad id", MINIMAL.replace("hello", "Hello_World"), "id"),
        ("bad semver", MINIMAL.replace("0.1.0", "one"), "version"),
        (
            "bad schema version",
            MINIMAL.replace("schema_version = 1", "schema_version = 2"),
            "schema_version",
        ),
        (
            "duplicate command id",
            with(
                "[[commands]]\nid = \"a\"\ntitle = \"A\"\n[[commands]]\nid = \"a\"\ntitle = \"B\"\n",
            ),
            "duplicate id",
        ),
        (
            "colon in command id",
            with("[[commands]]\nid = \"a:b\"\ntitle = \"A\"\n"),
            "[a-z0-9-]+",
        ),
        (
            "duplicate context server",
            with("[[context_servers]]\nid = \"s\"\n[[context_servers]]\nid = \"s\"\n"),
            "duplicate id",
        ),
        (
            "absolute theme path",
            with("themes = [\"/etc/theme.json\"]\n"),
            "relative",
        ),
        (
            "escaping theme path",
            with("icon_themes = [\"../x.json\"]\n"),
            "relative",
        ),
        (
            "unknown scope",
            with("[[commands]]\nid = \"a\"\ntitle = \"A\"\nscopes = [\"galaxy\"]\n"),
            "galaxy",
        ),
        (
            "unknown capability",
            with("capabilities = [{ kind = \"fs:write\" }]\n"),
            "fs:write",
        ),
        (
            "empty pattern",
            with("capabilities = [{ kind = \"kube:read\", contexts = [\"\"] }]\n"),
            "non-empty",
        ),
        (
            "no patterns",
            with("capabilities = [{ kind = \"kube:read\", contexts = [] }]\n"),
            "non-empty",
        ),
        (
            "empty command",
            with("capabilities = [{ kind = \"process:exec\", command = \" \" }]\n"),
            "non-empty",
        ),
    ];
    for (name, text, needle) in cases {
        let err = ExtensionManifest::parse(&text).expect_err(name);
        assert!(err.to_string().contains(needle), "{name}: {err}");
    }
}
