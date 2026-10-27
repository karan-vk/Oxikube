//! The file name of a pasted kubeconfig and the "stored here" test.

use std::path::Path;

use proptest::prelude::*;

use oxikube_domain::ErrorKind;

use super::super::{is_managed_stem, is_stored_in, pasted_file_name};

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

#[test]
fn the_names_cloud_discovery_manages_are_refused() {
    for name in ["aws", "AWS.yaml", "gcloud", "az.yml", " Az "] {
        let error = pasted_file_name(name).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Validation, "{name}");
        assert!(error.message().contains("reserved"), "{name}: {error}");
    }
    // Only the whole name is reserved: a longer one that starts the same is fine.
    for (name, file) in [("aws-prod", "aws-prod.yaml"), ("az.dev", "az.dev.yaml")] {
        assert_eq!(pasted_file_name(name).unwrap(), file, "{name}");
    }
    assert!(is_managed_stem("GCLOUD") && !is_managed_stem("gcp"));
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
