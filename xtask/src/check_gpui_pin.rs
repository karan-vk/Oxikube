//! GPUI pin alignment check.
//!
//! `gpui-component` is compiled against exactly one `gpui-pre` snapshot. The
//! table below is the source of truth for which versions pair; bump all of them
//! together in one PR (docs/adr/0003-gpui-dependency.md).

use anyhow::{Context, Result, bail};
use std::fs;

/// (gpui-pre version, gpui-component version, zed commit named by the snapshot)
const PAIRS: &[(&str, &str, &str)] = &[("0.3.7", "0.7.0", "1a28cff")];

const GPUI_PRE_PACKAGES: &[&str] = &["gpui-pre", "gpui-pre-platform", "gpui-pre-macros"];
const GPUI_KIT_PACKAGES: &[&str] = &["gpui-component", "gpui-base", "gpui-kit-assets"];

pub fn run() -> Result<()> {
    let text = fs::read_to_string("Cargo.toml").context("read workspace Cargo.toml")?;
    let doc: toml::Value = toml::from_str(&text)?;
    let deps = doc
        .get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| d.as_table())
        .context("[workspace.dependencies] missing")?;

    let mut pre_versions = Vec::new();
    let mut kit_versions = Vec::new();
    for (key, val) in deps {
        let tbl = match val.as_table() {
            Some(t) => t,
            None => continue,
        };
        let package = tbl.get("package").and_then(|p| p.as_str()).unwrap_or(key);
        let version = tbl
            .get("version")
            .and_then(|v| v.as_str())
            .or_else(|| val.as_str());
        if GPUI_PRE_PACKAGES.contains(&package) {
            pre_versions.push((package.to_string(), version.map(str::to_string)));
        } else if GPUI_KIT_PACKAGES.contains(&package) {
            kit_versions.push((package.to_string(), version.map(str::to_string)));
        }
    }

    let mut errors = Vec::new();
    let exact = |pkg: &str, v: &Option<String>, errors: &mut Vec<String>| -> Option<String> {
        match v {
            Some(v) if v.starts_with('=') => Some(v[1..].to_string()),
            Some(v) => {
                errors.push(format!(
                    "{pkg}: version `{v}` must be an exact `=x.y.z` pin"
                ));
                None
            }
            None => {
                errors.push(format!("{pkg}: missing version"));
                None
            }
        }
    };
    let pre: Vec<_> = pre_versions
        .iter()
        .filter_map(|(p, v)| exact(p, v, &mut errors))
        .collect();
    let kit: Vec<_> = kit_versions
        .iter()
        .filter_map(|(p, v)| exact(p, v, &mut errors))
        .collect();
    if pre.is_empty() {
        errors.push("no gpui-pre dependency found".into());
    }
    if pre.windows(2).any(|w| w[0] != w[1]) {
        errors.push(format!("gpui-pre-* versions differ: {pre:?}"));
    }
    if kit.windows(2).any(|w| w[0] != w[1]) {
        errors.push(format!("gpui-kit crate versions differ: {kit:?}"));
    }
    if let (Some(p), Some(k)) = (pre.first(), kit.first()) {
        match PAIRS.iter().find(|(pv, kv, _)| pv == p && kv == k) {
            Some((_, _, zed)) => println!("check-gpui-pin: gpui-pre ={p} ↔ gpui-component ={k} (snapshot of zed@{zed}) OK"),
            None => errors.push(format!(
                "gpui-pre ={p} and gpui-component ={k} are not a known pairing; update PAIRS in xtask/src/check_gpui_pin.rs and docs/adr/0003-gpui-dependency.md"
            )),
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        for e in &errors {
            eprintln!("error: {e}");
        }
        bail!("check-gpui-pin: {} problem(s)", errors.len())
    }
}
