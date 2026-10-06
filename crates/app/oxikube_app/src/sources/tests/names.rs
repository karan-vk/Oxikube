//! The file name of a pasted kubeconfig and the "stored here" test.

use std::path::Path;

use proptest::prelude::*;

use super::super::{is_stored_in, pasted_file_name};

#[test]
fn good_names_become_yaml_files() {
    for (name, file) in [
        ("prod", "prod.yaml"),
        ("  prod  ", "prod.yaml"),
        ("prod.yaml", "prod.yaml"),
        ("Prod.YML", "Prod.yaml"),
        ("eu-west_1", "eu-west_1.yaml"),
        ("v1.2.3", "v1.2.3.yaml"),
    ] {
        assert_eq!(pasted_file_name(name).unwrap(), file, "{name}");
    }
}

#[test]
fn stored_in_means_directly_inside_without_a_way_out() {
    let dir = Path::new("/config/kubeconfigs");
    assert!(is_stored_in(dir, Path::new("/config/kubeconfigs/a.yaml")));
    assert!(!is_stored_in(dir, Path::new("/config/kubeconfigs")));
    assert!(!is_stored_in(
        dir,
        Path::new("/config/kubeconfigs/sub/a.yaml")
    ));
    assert!(!is_stored_in(
        dir,
        Path::new("/config/kubeconfigs/../a.yaml")
    ));
    assert!(!is_stored_in(
        dir,
        Path::new("/config/kubeconfigs-other/a.yaml")
    ));
    assert!(!is_stored_in(dir, Path::new("/elsewhere/a.yaml")));
}

proptest! {
    /// Whatever is typed, an accepted name is one plain file name: it joins onto the directory
    /// and stays directly inside it.
    #[test]
    fn an_accepted_name_never_leaves_the_directory(name in ".{0,80}") {
        if let Ok(file) = pasted_file_name(&name) {
            let dir = Path::new("/config/kubeconfigs");
            let path = dir.join(&file);
            prop_assert!(is_stored_in(dir, &path), "{name:?} -> {file:?}");
            prop_assert!(file.ends_with(".yaml"));
            prop_assert!(!file.starts_with('.'));
        }
    }
}
