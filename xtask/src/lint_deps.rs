//! Dependency-direction lint.
//!
//! The workspace is organised by hexagonal layer (directory under `crates/`).
//! The rules, mirrored in docs/ARCHITECTURE.md:
//!
//! | layer    | may depend on internal layers        | banned external crates                     |
//! |----------|--------------------------------------|--------------------------------------------|
//! | domain   | (none)                               | gpui*, kube*, k8s-openapi, tokio, reqwest  |
//! | ports    | domain                               | gpui*, kube*, k8s-openapi, gpui-component  |
//! | app      | domain, ports                        | gpui*, kube*, k8s-openapi, gpui-component  |
//! | adapters | domain, ports                        | gpui*, gpui-component                      |
//! | platform | domain, ports, platform              | kube*, gpui-component (except oxikube_ui)  |
//! | ui       | domain, ports, app, platform, ui     | kube*, k8s-openapi; gpui-component only in oxikube_ui |
//! | testing  | domain, ports                        | gpui-component                             |
//! | bins     | anything                             | –                                          |
//!
//! `chrono`, `serde_yaml` and `serde_yml` are banned in every layer (jiff + serde-saphyr instead).
//!
//! Only direct `[dependencies]` are checked (dev-dependencies are exempt so tests can use
//! testkit and fixtures freely). Run with `cargo xtask lint-deps`.

use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind, Metadata, MetadataCommand, Package};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    Domain,
    Ports,
    App,
    Adapters,
    Platform,
    Ui,
    Testing,
    Bins,
    Xtask,
}

impl Layer {
    fn of(pkg: &Package, root: &std::path::Path) -> Option<Layer> {
        let rel = pkg.manifest_path.as_std_path().strip_prefix(root).ok()?;
        let mut parts = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string());
        match parts.next().as_deref() {
            Some("bins") => Some(Layer::Bins),
            Some("xtask") => Some(Layer::Xtask),
            Some("crates") => match parts.next().as_deref() {
                Some("domain") => Some(Layer::Domain),
                Some("ports") => Some(Layer::Ports),
                Some("app") => Some(Layer::App),
                Some("adapters") => Some(Layer::Adapters),
                Some("platform") => Some(Layer::Platform),
                Some("ui") => Some(Layer::Ui),
                Some("testing") => Some(Layer::Testing),
                _ => None,
            },
            _ => None,
        }
    }

    fn allowed_internal(self) -> &'static [Layer] {
        use Layer::*;
        match self {
            Domain => &[],
            Ports => &[Domain],
            App => &[Domain, Ports],
            Adapters => &[Domain, Ports],
            Platform => &[Domain, Ports, Platform],
            Ui => &[Domain, Ports, App, Platform, Ui],
            Testing => &[Domain, Ports],
            Bins | Xtask => &[Domain, Ports, App, Adapters, Platform, Ui, Testing],
        }
    }

    /// External crate-name prefixes that must not appear in this layer's direct deps.
    fn banned_external(self) -> &'static [&'static str] {
        use Layer::*;
        const EVERYWHERE: &[&str] = &["chrono", "serde_yaml", "serde_yml"];
        match self {
            Domain => &[
                "gpui",
                "kube",
                "k8s-openapi",
                "k8s-metrics",
                "tokio",
                "reqwest",
                "rusqlite",
                "wasmtime",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            Ports => &[
                "gpui",
                "kube",
                "k8s-openapi",
                "k8s-metrics",
                "reqwest",
                "rusqlite",
                "wasmtime",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            App => &[
                "gpui",
                "kube",
                "k8s-openapi",
                "k8s-metrics",
                "reqwest",
                "rusqlite",
                "wasmtime",
                "agent-client-protocol",
                "rmcp",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            Adapters => &["gpui", "chrono", "serde_yaml", "serde_yml"],
            Platform => &[
                "kube",
                "k8s-openapi",
                "gpui-component",
                "gpui-base",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            Ui => &[
                "kube",
                "k8s-openapi",
                "k8s-metrics",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            Testing => &[
                "gpui-component",
                "gpui-base",
                "chrono",
                "serde_yaml",
                "serde_yml",
            ],
            Bins | Xtask => EVERYWHERE,
        }
    }
}

/// Thin I/O shell: ask cargo for the workspace metadata, run [`check`], report.
pub fn run() -> Result<()> {
    let meta = MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
    let errors = check(&meta);
    if errors.is_empty() {
        println!(
            "lint-deps: {} workspace crates OK",
            meta.workspace_packages().len()
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("error: {e}");
        }
        bail!("lint-deps: {} violation(s)", errors.len())
    }
}

/// Pure check over parsed `cargo metadata` output. Returns one message per violation
/// (empty when the dependency graph obeys the layer rules).
pub fn check(meta: &Metadata) -> Vec<String> {
    let root = meta.workspace_root.as_std_path();
    let members: BTreeMap<_, _> = meta
        .workspace_packages()
        .into_iter()
        .map(|p| (p.name.to_string(), p))
        .collect();

    let mut errors = Vec::new();
    for (name, pkg) in &members {
        let Some(layer) = Layer::of(pkg, root) else {
            errors.push(format!(
                "{name}: not under a known layer directory ({})",
                pkg.manifest_path
            ));
            continue;
        };
        for dep in pkg
            .dependencies
            .iter()
            .filter(|d| d.kind == DependencyKind::Normal)
        {
            let dep_name = dep.name.as_str();
            if let Some(target) = members.get(dep_name) {
                let target_layer = Layer::of(target, root).unwrap_or(Layer::Bins);
                if !layer.allowed_internal().contains(&target_layer) {
                    errors.push(format!(
                        "{name} ({layer:?}) must not depend on {dep_name} ({target_layer:?})"
                    ));
                }
                if target_layer == Layer::Ui && dep_name != "oxikube_ui" && layer == Layer::Platform
                {
                    errors.push(format!(
                        "{name} (platform) must not depend on ui crate {dep_name}"
                    ));
                }
            } else {
                // gpui-component is only allowed inside oxikube_ui (the wrapper crate).
                let gpui_component = dep_name == "gpui-component"
                    || dep_name == "gpui-base"
                    || dep_name == "gpui-kit";
                if gpui_component && name != "oxikube_ui" && layer != Layer::Bins {
                    errors.push(format!(
                        "{name}: gpui-component may only be imported by oxikube_ui (views use the oxikube_ui wrapper)"
                    ));
                    continue;
                }
                for banned in layer.banned_external() {
                    if dep_name == *banned
                        || dep_name.starts_with(&format!("{banned}-"))
                        || dep_name.starts_with(&format!("{banned}_"))
                    {
                        errors.push(format!(
                            "{name} ({layer:?}) must not depend on external crate {dep_name}"
                        ));
                    }
                }
            }
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const CLEAN: &str = include_str!("../tests/fixtures/clean.json");
    const VIOLATIONS: &str = include_str!("../tests/fixtures/violations.json");

    fn parse(json: &str) -> Metadata {
        serde_json::from_str(json).expect("fixture is valid `cargo metadata` JSON")
    }

    /// Load the clean fixture, let `edit` mutate its raw JSON, and run the check.
    fn check_edited(edit: impl FnOnce(&mut Value)) -> Vec<String> {
        let mut value: Value = serde_json::from_str(CLEAN).unwrap();
        edit(&mut value);
        check(&serde_json::from_value(value).unwrap())
    }

    fn package<'a>(meta: &'a mut Value, name: &str) -> &'a mut Value {
        meta["packages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["name"] == name)
            .unwrap_or_else(|| panic!("fixture has no package {name}"))
    }

    /// Add a dependency edge (`kind` is `None` for a normal dependency, `Some("dev")`, ...).
    fn add_dep(meta: &mut Value, from: &str, to: &str, kind: Option<&str>) {
        let dep = json!({
            "name": to, "source": null, "req": "*", "kind": kind, "rename": null,
            "optional": false, "uses_default_features": true, "features": [],
            "target": null, "registry": null
        });
        package(meta, from)["dependencies"]
            .as_array_mut()
            .unwrap()
            .push(dep);
    }

    fn assert_one(errors: &[String], needle: &str) {
        assert_eq!(
            errors.len(),
            1,
            "expected exactly one violation: {errors:#?}"
        );
        assert!(
            errors[0].contains(needle),
            "{:?} lacks {needle:?}",
            errors[0]
        );
    }

    #[test]
    fn clean_graph_passes() {
        let meta = parse(CLEAN);
        assert_eq!(meta.workspace_packages().len(), 10);
        assert_eq!(check(&meta), Vec::<String>::new());
    }

    #[test]
    fn violations_fixture_reports_every_rule() {
        let errors = check(&parse(VIOLATIONS));
        let expected = [
            "oxikube_domain (Domain) must not depend on oxikube_ports (Ports)",
            "oxikube_domain (Domain) must not depend on external crate chrono",
            "oxikube_ports (Ports) must not depend on oxikube_app (App)",
            "oxikube_app (App) must not depend on external crate gpui-pre",
            "oxikube_app (App) must not depend on external crate kube",
            "oxikube_app (App) must not depend on oxikube_kube (Adapters)",
            "oxikube_kube (Adapters) must not depend on oxikube_app (App)",
            "oxikube_kube (Adapters) must not depend on oxikube_logs_ui (Ui)",
            "oxikube_logs_ui (Ui) must not depend on oxikube_kube (Adapters)",
            "oxikube_logs_ui: gpui-component may only be imported by oxikube_ui",
            "oxikube_runtime (Platform) must not depend on oxikube_ui (Ui)",
            "oxikube_testkit (Testing) must not depend on external crate serde_yaml",
            "oxikube_testkit (Testing) must not depend on external crate serde_yml",
        ];
        for needle in expected {
            assert!(
                errors.iter().any(|e| e.contains(needle)),
                "missing violation {needle:?} in {errors:#?}"
            );
        }
        assert_eq!(errors.len(), expected.len(), "{errors:#?}");
    }

    #[test]
    fn domain_may_not_depend_on_any_internal_crate() {
        for target in [
            "oxikube_ports",
            "oxikube_app",
            "oxikube_kube",
            "oxikube_ui",
            "oxikube_testkit",
        ] {
            let errors = check_edited(|m| add_dep(m, "oxikube_domain", target, None));
            assert!(
                errors.iter().any(|e| e.starts_with(&format!(
                    "oxikube_domain (Domain) must not depend on {target}"
                ))),
                "domain -> {target} should fail: {errors:#?}"
            );
        }
    }

    #[test]
    fn domain_bans_runtime_and_framework_crates() {
        for banned in [
            "gpui-pre",
            "kube",
            "k8s-openapi",
            "tokio",
            "reqwest",
            "rusqlite",
            "wasmtime",
        ] {
            let errors = check_edited(|m| add_dep(m, "oxikube_domain", banned, None));
            assert_one(&errors, &format!("external crate {banned}"));
        }
    }

    #[test]
    fn ports_may_only_depend_on_domain() {
        // domain is fine (already in the clean fixture); everything else is not.
        for (target, layer) in [
            ("oxikube_app", "App"),
            ("oxikube_kube", "Adapters"),
            ("oxikube_runtime", "Platform"),
            ("oxikube_ui", "Ui"),
            ("oxikube_testkit", "Testing"),
        ] {
            let errors = check_edited(|m| add_dep(m, "oxikube_ports", target, None));
            assert_one(
                &errors,
                &format!("oxikube_ports (Ports) must not depend on {target} ({layer})"),
            );
        }
    }

    #[test]
    fn app_may_not_depend_on_gpui_kube_or_adapters() {
        let errors = check_edited(|m| add_dep(m, "oxikube_app", "gpui-pre", None));
        assert_one(
            &errors,
            "oxikube_app (App) must not depend on external crate gpui-pre",
        );

        let errors = check_edited(|m| add_dep(m, "oxikube_app", "kube", None));
        assert_one(&errors, "external crate kube");

        // prefix matching: kube-runtime and k8s-openapi are covered too.
        let errors = check_edited(|m| add_dep(m, "oxikube_app", "kube-runtime", None));
        assert_one(&errors, "external crate kube-runtime");
        let errors = check_edited(|m| add_dep(m, "oxikube_app", "k8s-openapi", None));
        assert_one(&errors, "external crate k8s-openapi");

        let errors = check_edited(|m| add_dep(m, "oxikube_app", "oxikube_kube", None));
        assert_one(
            &errors,
            "oxikube_app (App) must not depend on oxikube_kube (Adapters)",
        );
    }

    #[test]
    fn app_may_not_depend_on_ui_or_platform_crates() {
        let errors = check_edited(|m| add_dep(m, "oxikube_app", "oxikube_ui", None));
        assert_one(
            &errors,
            "oxikube_app (App) must not depend on oxikube_ui (Ui)",
        );
        let errors = check_edited(|m| add_dep(m, "oxikube_app", "oxikube_runtime", None));
        assert_one(
            &errors,
            "oxikube_app (App) must not depend on oxikube_runtime (Platform)",
        );
    }

    #[test]
    fn adapters_may_not_depend_on_app_or_ui() {
        let errors = check_edited(|m| add_dep(m, "oxikube_kube", "oxikube_app", None));
        assert_one(
            &errors,
            "oxikube_kube (Adapters) must not depend on oxikube_app (App)",
        );
        let errors = check_edited(|m| add_dep(m, "oxikube_kube", "oxikube_logs_ui", None));
        assert_one(
            &errors,
            "oxikube_kube (Adapters) must not depend on oxikube_logs_ui (Ui)",
        );
        let errors = check_edited(|m| add_dep(m, "oxikube_kube", "gpui-pre", None));
        assert_one(
            &errors,
            "oxikube_kube (Adapters) must not depend on external crate gpui-pre",
        );
    }

    #[test]
    fn ui_may_not_depend_on_adapters() {
        let errors = check_edited(|m| add_dep(m, "oxikube_logs_ui", "oxikube_kube", None));
        assert_one(
            &errors,
            "oxikube_logs_ui (Ui) must not depend on oxikube_kube (Adapters)",
        );
        let errors = check_edited(|m| add_dep(m, "oxikube_logs_ui", "kube", None));
        assert_one(
            &errors,
            "oxikube_logs_ui (Ui) must not depend on external crate kube",
        );
    }

    #[test]
    fn platform_may_not_depend_on_ui_crates() {
        let errors = check_edited(|m| add_dep(m, "oxikube_runtime", "oxikube_logs_ui", None));
        assert!(
            errors
                .iter()
                .any(|e| e.contains("must not depend on ui crate oxikube_logs_ui")),
            "{errors:#?}"
        );
    }

    #[test]
    fn gpui_component_is_confined_to_oxikube_ui() {
        // oxikube_ui itself and the bin (placeholder window) may use it: the clean fixture has both.
        assert!(check(&parse(CLEAN)).is_empty());
        for dep in ["gpui-component", "gpui-base", "gpui-kit"] {
            let errors = check_edited(|m| add_dep(m, "oxikube_logs_ui", dep, None));
            assert_one(&errors, "gpui-component may only be imported by oxikube_ui");
        }
        let errors = check_edited(|m| add_dep(m, "oxikube_runtime", "gpui-component", None));
        assert_one(
            &errors,
            "oxikube_runtime: gpui-component may only be imported by oxikube_ui",
        );
        let errors = check_edited(|m| add_dep(m, "oxikube_testkit", "gpui-component", None));
        assert_one(
            &errors,
            "oxikube_testkit: gpui-component may only be imported by oxikube_ui",
        );
    }

    #[test]
    fn banned_crates_fail_in_every_layer() {
        let crates = [
            "oxikube_domain",
            "oxikube_ports",
            "oxikube_app",
            "oxikube_kube",
            "oxikube_runtime",
            "oxikube_ui",
            "oxikube_logs_ui",
            "oxikube_testkit",
            "oxikube",
            "xtask",
        ];
        for pkg in crates {
            for banned in ["chrono", "serde_yaml", "serde_yml"] {
                let errors = check_edited(|m| add_dep(m, pkg, banned, None));
                assert_one(&errors, &format!("{pkg} ("));
                assert_one(&errors, &format!("external crate {banned}"));
            }
        }
    }

    #[test]
    fn dev_and_build_dependencies_are_exempt() {
        // The clean fixture already has oxikube_app -> oxikube_testkit and domain -> tokio as dev-deps.
        for kind in ["dev", "build"] {
            let errors = check_edited(|m| {
                add_dep(m, "oxikube_domain", "oxikube_kube", Some(kind));
                add_dep(m, "oxikube_domain", "chrono", Some(kind));
                add_dep(m, "oxikube_app", "kube", Some(kind));
                add_dep(m, "oxikube_logs_ui", "gpui-component", Some(kind));
            });
            assert_eq!(errors, Vec::<String>::new());
        }
    }

    #[test]
    fn bins_may_depend_on_anything_internal() {
        let errors = check_edited(|m| {
            for target in [
                "oxikube_domain",
                "oxikube_ports",
                "oxikube_testkit",
                "oxikube_logs_ui",
            ] {
                add_dep(m, "oxikube", target, None);
            }
        });
        assert_eq!(errors, Vec::<String>::new());
    }

    #[test]
    fn crate_outside_a_layer_directory_is_an_error() {
        let errors = check_edited(|m| {
            package(m, "oxikube_logs_ui")["manifest_path"] =
                json!("/ws/crates/misc/oxikube_logs_ui/Cargo.toml");
        });
        assert_one(
            &errors,
            "oxikube_logs_ui: not under a known layer directory",
        );
    }
}
