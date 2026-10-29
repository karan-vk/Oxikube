//! The manifest loader over the in-memory filesystem: recursion, order, filters, caps.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxikube_domain::ErrorKind;
use oxikube_ports::{DirEntry, EntryKind, FsPort};
use oxikube_testkit::FakeFsPort;

use crate::apply::{LoadLimits, load_path, load_text};

fn cm(name: &str) -> String {
    format!("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}\n")
}

fn names(docs: &[oxikube_domain::manifests::ManifestDoc]) -> Vec<String> {
    docs.iter()
        .map(|d| {
            d.json["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

fn fs(fake: FakeFsPort) -> Arc<dyn FsPort> {
    Arc::new(fake)
}

#[tokio::test]
async fn walks_recursively_in_name_order_with_filters() {
    let fake = FakeFsPort::new()
        .with_file("/m/b.yaml", cm("b"))
        .with_file(
            "/m/a/z.JSON",
            r#"{"apiVersion":"v1","kind":"Namespace","metadata":{"name":"z"}}"#,
        )
        .with_file("/m/a/y.yml", cm("yy"))
        .with_file("/m/a/notes.txt", "not a manifest")
        .with_file("/m/.git/config.yaml", cm("hidden"))
        .with_file("/m/c/deep/d.yaml", format!("{}---\n{}", cm("d1"), cm("d2")))
        .with_file("/m/kustomization.yaml", "resources: [a]\n");
    let loaded = load_path(&fs(fake), Path::new("/m"), LoadLimits::default())
        .await
        .expect("loads");
    assert_eq!(names(&loaded.docs), ["yy", "z", "b", "d1", "d2"]);
    assert_eq!(loaded.files, 4);
    assert!(loaded.errors.is_empty());
    assert_eq!(loaded.notes.len(), 1);
    assert!(loaded.notes[0].path.ends_with("kustomization.yaml"));
    assert!(loaded.docs[3].source.as_str().ends_with("c/deep/d.yaml"));
    assert_eq!((loaded.docs[3].line, loaded.docs[4].line), (1, 6));
}

#[tokio::test]
async fn a_single_file_loads_and_a_missing_path_is_not_found() {
    let fake = FakeFsPort::new().with_file("/m/one.yaml", cm("one"));
    let fs = fs(fake);
    let loaded = load_path(&fs, Path::new("/m/one.yaml"), LoadLimits::default())
        .await
        .expect("loads");
    assert_eq!(names(&loaded.docs), ["one"]);
    let error = load_path(&fs, Path::new("/m/none.yaml"), LoadLimits::default())
        .await
        .expect_err("missing");
    assert_eq!(error.kind(), ErrorKind::NotFound);
}

#[tokio::test]
async fn a_bad_file_does_not_stop_the_others() {
    let fake = FakeFsPort::new()
        .with_file("/m/a.yaml", cm("a"))
        .with_file("/m/b.yaml", "foo: [unclosed\n")
        .with_file("/m/c.yaml", vec![0xff, 0xfe, 0x00])
        .with_file("/m/d.yaml", cm("d"));
    let loaded = load_path(&fs(fake), Path::new("/m"), LoadLimits::default())
        .await
        .expect("loads");
    assert_eq!(names(&loaded.docs), ["a", "d"]);
    assert_eq!(loaded.errors.len(), 2);
    assert!(loaded.errors[0].source.as_str().ends_with("b.yaml"));
    assert!(loaded.errors[1].message.contains("UTF-8"));
}

#[tokio::test]
async fn limits_stop_a_runaway_load() {
    let mut fake = FakeFsPort::new();
    for i in 0..5 {
        fake = fake.with_file(format!("/m/f{i}.yaml"), cm("x"));
    }
    let fs = fs(fake);
    let few = LoadLimits {
        max_files: 3,
        ..LoadLimits::default()
    };
    let error = load_path(&fs, Path::new("/m"), few)
        .await
        .expect_err("too many");
    assert_eq!(error.kind(), ErrorKind::Validation);
    let total = LoadLimits {
        max_total_bytes: 100,
        ..LoadLimits::default()
    };
    assert!(load_path(&fs, Path::new("/m"), total).await.is_err());
    // A file over the single-file limit is reported, the rest loads.
    let big = LoadLimits {
        max_file_bytes: 10,
        ..LoadLimits::default()
    };
    let loaded = load_path(&fs, Path::new("/m"), big).await.expect("loads");
    assert!(loaded.docs.is_empty());
    assert_eq!(loaded.errors.len(), 5);
}

#[tokio::test]
async fn depth_is_capped_with_a_note() {
    let fake = FakeFsPort::new()
        .with_file("/m/a.yaml", cm("a"))
        .with_file("/m/d1/d2/d3/deep.yaml", cm("deep"));
    let limits = LoadLimits {
        max_depth: 2,
        ..LoadLimits::default()
    };
    let loaded = load_path(&fs(fake), Path::new("/m"), limits)
        .await
        .expect("loads");
    assert_eq!(names(&loaded.docs), ["a"]);
    assert!(loaded.notes[0].message.contains("deeper"));
}

#[tokio::test]
async fn symlinks_are_not_followed() {
    let fake = FakeFsPort::new().with_file("/m/a.yaml", cm("a"));
    // The listing of /m shows a link back to /m: following it would loop forever.
    fake.script().list.push_ok(vec![
        DirEntry {
            path: PathBuf::from("/m/a.yaml"),
            kind: EntryKind::File,
            size: Some(10),
        },
        DirEntry {
            path: PathBuf::from("/m/loop"),
            kind: EntryKind::Symlink,
            size: None,
        },
    ]);
    let loaded = load_path(&fs(fake), Path::new("/m"), LoadLimits::default())
        .await
        .expect("loads");
    assert_eq!(names(&loaded.docs), ["a"]);
    assert_eq!(loaded.notes.len(), 1);
    assert!(loaded.notes[0].message.contains("symbolic link"));
}

#[test]
fn text_loads_under_its_label() {
    let loaded = load_text(&format!("{}---\n{}", cm("a"), cm("b")), "clipboard");
    assert_eq!(names(&loaded.docs), ["a", "b"]);
    assert_eq!(loaded.docs[1].source.as_str(), "clipboard");
}
