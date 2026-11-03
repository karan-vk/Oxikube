use super::*;
use crate::icons::read::read_icon_theme_file;
use crate::icons::theme::IconThemeFamily;
use std::path::Path;

fn fixture(file: &str) -> Arc<IconTheme> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/icon_themes");
    let source = read_icon_theme_file(&root.join(file), &root).unwrap();
    let mut family = IconThemeFamily::mount(source);
    Arc::new(family.themes.remove(0))
}

fn lucide(icon: IconName) -> SharedString {
    SharedString::from(icon.path())
}

#[test]
fn lucide_resolves_every_kind_and_status_to_its_default() {
    let icons = IconResolver::lucide();
    for icon in IconName::ALL {
        assert_eq!(icons.icon(*icon), lucide(*icon));
    }
    assert_eq!(icons.kind("", "Pod"), lucide(IconName::Box));
    assert_eq!(icons.kind("apps", "Deployment"), lucide(IconName::Boxes));
    assert_eq!(
        icons.kind("example.com", "Widget"),
        lucide(IconName::FileCode)
    );
    assert_eq!(icons.status("Running"), lucide(IconName::CircleCheck));
    assert_eq!(icons.status("CrashLoopBackOff"), lucide(IconName::CircleX));
    assert_eq!(icons.file("main.rs"), None);
}

#[test]
fn kind_and_status_keys_override_and_the_rest_falls_back() {
    let icons = IconResolver::new(fixture("kube.json"));
    let pod = icons.kind("", "Pod");
    assert!(
        pod.starts_with("icon-themes/") && pod.ends_with("/kube/pod.svg"),
        "{pod}"
    );
    // `kind:<Kind>` matches any group; `kind:<group>/<Kind>` only its own.
    assert!(icons.kind("", "Service").ends_with("/kube/service.svg"));
    assert!(
        icons
            .kind("apps", "Deployment")
            .ends_with("/kube/deployment.svg")
    );
    assert_eq!(
        icons.kind("example.com", "Deployment"),
        lucide(IconName::FileCode)
    );
    assert!(
        icons
            .kind("argoproj.io", "Application")
            .ends_with("/kube/app.svg")
    );
    // Not in the theme: Lucide.
    assert_eq!(icons.kind("", "ConfigMap"), lucide(IconName::FileText));
    assert!(icons.status("Running").ends_with("/kube/running.svg"));
    assert!(icons.status("Failed").ends_with("/kube/failed.svg"));
    assert_eq!(icons.status("Pending"), lucide(IconName::Clock));
    // `icon:server` changes the icon itself, so Node (whose default is Server) follows it.
    assert!(icons.icon(IconName::Server).ends_with("/kube/server.svg"));
    assert_eq!(icons.kind("", "Node"), icons.icon(IconName::Server));
    assert_eq!(icons.icon(IconName::Box), lucide(IconName::Box));
}

#[test]
fn a_zed_theme_drives_chevrons_folders_and_files_and_leaves_kinds_alone() {
    let icons = IconResolver::new(fixture("zed-sample.json"));
    assert!(
        icons
            .icon(IconName::ChevronRight)
            .ends_with("/zed-sample/icons/chevron-right.svg")
    );
    assert!(
        icons
            .icon(IconName::ChevronDown)
            .ends_with("/zed-sample/icons/chevron-down.svg")
    );
    assert!(
        icons
            .icon(IconName::Folder)
            .ends_with("/zed-sample/icons/folder.svg")
    );
    assert_eq!(icons.kind("", "Pod"), lucide(IconName::Box));
    assert_eq!(icons.status("Running"), lucide(IconName::CircleCheck));
    let file = |name: &str| icons.file(name).map(|path| path.to_string());
    assert!(file("Dockerfile").is_some_and(|p| p.ends_with("docker.svg")));
    assert!(file("main.rs").is_some_and(|p| p.ends_with("rust.svg")));
    assert!(file("values.prod.yml").is_some_and(|p| p.ends_with("yaml.svg")));
    assert_eq!(file("notes.txt"), None);
}

#[test]
fn resolved_paths_are_served_by_the_asset_source() {
    use gpui::AssetSource as _;
    let icons = IconResolver::new(fixture("kube.json"));
    let svg = oxikube_assets::Assets
        .load(&icons.kind("", "Pod"))
        .unwrap()
        .expect("the mounted file is served");
    assert!(svg.starts_with(b"<svg"));
}
