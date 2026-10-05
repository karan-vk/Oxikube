//! One theme entry: `style` + the optional `oxikube` block -> [`ThemeTokens`].

use super::report::ImportReport;
use super::syntax::{accents, color_at, players, syntax};
use super::table::{OXIKUBE, cluster_tab_index, color_slot};
use crate::appearance::Appearance;
use crate::tokens::{ThemeTokens, derive_colors, derive_oxikube};
use serde_json::{Map, Value};

/// Builds the tokens for one theme over `base`.
///
/// Keys the file does not set keep `base`'s value (as Zed's `refine_theme` falls back to its
/// fallback theme); invalid values are reported and keep it too. The derived colours and the
/// `oxikube` defaults are computed from the result, then explicit `oxikube` keys win.
pub(super) fn build_theme(
    name: &str,
    appearance: Appearance,
    style: &Map<String, Value>,
    oxikube: Option<&Map<String, Value>>,
    base: ThemeTokens,
    report: &mut ImportReport,
) -> ThemeTokens {
    let mut tokens = base;
    tokens.name = name.to_owned();
    tokens.appearance = appearance;

    for (key, value) in style {
        if let Some(slot) = color_slot(key) {
            if let Some(color) = color_at(value, name, key, report) {
                *slot(&mut tokens) = color;
            }
            continue;
        }
        match key.as_str() {
            "players" => {
                if let Some(players) = players(value, name, report).filter(|p| !p.is_empty()) {
                    tokens.players = players;
                }
            }
            "accents" => {
                if let Some(accents) = accents(value, name, report).filter(|a| !a.is_empty()) {
                    tokens.accents = accents;
                }
            }
            "syntax" => {
                if let Some(merged) = syntax(value, name, &tokens.syntax, report) {
                    tokens.syntax = merged;
                }
            }
            _ => {
                tracing::debug!(theme = name, key, "ignoring unmapped theme key");
                report.unknown_keys.insert(key.clone());
            }
        }
    }

    derive_colors(&mut tokens);
    tokens.oxikube = derive_oxikube(&tokens);
    if let Some(block) = oxikube {
        apply_oxikube_block(&mut tokens, name, block, report);
    }
    tokens
}

/// Explicit `oxikube` keys over the derived defaults.
fn apply_oxikube_block(
    tokens: &mut ThemeTokens,
    theme: &str,
    block: &Map<String, Value>,
    report: &mut ImportReport,
) {
    for (key, value) in block {
        let key_path = format!("oxikube.{key}");
        if let Some((_, slot)) = OXIKUBE.iter().find(|(name, _)| name == key) {
            if let Some(color) = color_at(value, theme, &key_path, report) {
                *slot(&mut tokens.oxikube) = color;
            }
        } else if let Some(index) = cluster_tab_index(key) {
            if let Some(color) = color_at(value, theme, &key_path, report) {
                tokens.oxikube.cluster_tabs[index] = color;
            }
        } else {
            tracing::debug!(theme, key, "ignoring unknown oxikube key");
            report.unknown_keys.insert(key_path);
        }
    }
}
