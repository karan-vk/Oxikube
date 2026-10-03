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
use cargo_metadata::{DependencyKind, MetadataCommand, Package};
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

pub fn run() -> Result<()> {
    let meta = MetadataCommand::new()
        .no_deps()
        .exec()
        .context("cargo metadata")?;
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

    if errors.is_empty() {
        println!("lint-deps: {} workspace crates OK", members.len());
        Ok(())
    } else {
        for e in &errors {
            eprintln!("error: {e}");
        }
        bail!("lint-deps: {} violation(s)", errors.len())
    }
}
