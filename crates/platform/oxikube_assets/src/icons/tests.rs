use super::IconName;
use std::collections::HashSet;

#[test]
fn paths_are_unique_and_resolve_back() {
    let mut seen = HashSet::new();
    for icon in IconName::ALL {
        assert!(seen.insert(icon.path()), "duplicate path {}", icon.path());
        assert_eq!(IconName::from_path(icon.path()), Some(*icon));
    }
}

#[test]
fn every_icon_embeds_an_svg() {
    for icon in IconName::ALL {
        let svg = std::str::from_utf8(icon.svg()).expect("svg is utf-8");
        assert!(svg.contains("<svg"), "{} is not an svg", icon.path());
    }
}

#[test]
fn unknown_paths_do_not_resolve() {
    assert_eq!(IconName::from_path("icons/not-a-real-icon.svg"), None);
    assert_eq!(IconName::from_path(""), None);
}

#[test]
fn index_is_the_position_in_all_and_stems_resolve_back() {
    for (ix, icon) in IconName::ALL.iter().enumerate() {
        assert_eq!(icon.index(), ix);
        assert_eq!(IconName::from_stem(icon.stem()), Some(*icon));
        assert_eq!(icon.path(), format!("icons/{}.svg", icon.stem()));
    }
    assert_eq!(IconName::from_stem("not-an-icon"), None);
}
