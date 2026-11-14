use super::*;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn names(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap().to_string_lossy().into_owned());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn command_kind_writes_the_expected_files_with_the_id_substituted() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("my-ext");
    scaffold("my-ext", Kind::Command, &dir, &repo()).unwrap();
    assert_eq!(
        names(&dir),
        [
            ".cargo/config.toml",
            ".gitignore",
            "Cargo.toml",
            "README.md",
            "build.sh",
            "extension.toml",
            "src/lib.rs"
        ]
    );
    let manifest = std::fs::read_to_string(dir.join("extension.toml")).unwrap();
    assert!(manifest.contains("id = \"my-ext\""));
    let table: toml::Table = manifest.parse().unwrap();
    assert_eq!(table["schema_version"].as_integer(), Some(1));
    assert_eq!(table["lib"]["kind"].as_str(), Some("rust"));
    assert!(
        std::fs::read_to_string(dir.join("src/lib.rs"))
            .unwrap()
            .contains("struct MyExt;")
    );
    let cargo: toml::Table = std::fs::read_to_string(dir.join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(cargo["package"]["name"].as_str(), Some("my-ext"));
    assert!(
        cargo["dependencies"]["oxikube_extension_api"]["path"]
            .as_str()
            .unwrap()
            .ends_with(API_CRATE)
    );
    assert!(
        !std::fs::read_to_string(dir.join("build.sh"))
            .unwrap()
            .contains("{{")
    );
}

#[test]
fn mcp_and_theme_kinds_declare_their_contribution() {
    let tmp = tempfile::tempdir().unwrap();
    scaffold("srv", Kind::Mcp, &tmp.path().join("srv"), &repo()).unwrap();
    let manifest = std::fs::read_to_string(tmp.path().join("srv/extension.toml")).unwrap();
    let table: toml::Table = manifest.parse().unwrap();
    assert_eq!(table["context_servers"][0]["id"].as_str(), Some("srv"));

    scaffold("pretty", Kind::Theme, &tmp.path().join("pretty"), &repo()).unwrap();
    assert_eq!(
        names(&tmp.path().join("pretty")),
        [
            ".gitignore",
            "README.md",
            "extension.toml",
            "themes/pretty.json"
        ]
    );
    let table: toml::Table = std::fs::read_to_string(tmp.path().join("pretty/extension.toml"))
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(table["themes"][0].as_str(), Some("themes/pretty.json"));
    assert!(!table.contains_key("lib"));
}

#[test]
fn inside_the_repo_the_sdk_is_a_relative_path() {
    let repo = repo();
    let dep = api_dependency(&repo.join("extensions/foo"), &repo);
    assert_eq!(dep, format!("{{ path = \"../../{API_CRATE}\" }}"));
}

#[test]
fn invalid_ids_and_existing_directories_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    for bad in ["", "Upper", "a_b", "-a", "a-", "a b", "a/b"] {
        assert!(
            scaffold(bad, Kind::Command, &tmp.path().join("x"), &repo()).is_err(),
            "{bad}"
        );
    }
    assert!(scaffold("ok", Kind::Command, tmp.path(), &repo()).is_err());
}

#[test]
fn type_names_are_valid_rust_identifiers() {
    assert_eq!(type_name("hello-world"), "HelloWorld");
    assert_eq!(type_name("1pass"), "Ext1pass");
}

#[test]
fn every_toml_block_in_the_authoring_guide_parses() {
    let guide = std::fs::read_to_string(repo().join("docs/extensions.md")).unwrap();
    let mut blocks = 0;
    for chunk in guide.split("```toml\n").skip(1) {
        let block = chunk.split("```").next().unwrap();
        block
            .parse::<toml::Table>()
            .unwrap_or_else(|e| panic!("{e}\n{block}"));
        blocks += 1;
    }
    assert!(blocks >= 3);
}
