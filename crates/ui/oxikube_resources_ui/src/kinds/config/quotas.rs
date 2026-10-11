//! ResourceQuota (used against hard, as bars) and LimitRange (limits and defaults, as tables).

use oxikube_domain::Resource;
use oxikube_domain::view::{LimitRangeSummary, QuotaLine, QuotaSummary};

use super::model::{Panel, Row};

/// A quota's lines as bars: resources first, then the object counts (`pods`, `count/...`).
pub(super) fn resource_quota(res: &Resource) -> Option<Panel> {
    let quota = QuotaSummary::from_resource(res).ok()?;
    let bar = |line: &QuotaLine| Row::Bar {
        label: line.resource.to_string(),
        text: line.text(),
        percent: line.percent(),
        level: line.level(),
    };
    let (counts, resources): (Vec<&QuotaLine>, Vec<&QuotaLine>) =
        quota.lines.iter().partition(|line| line.is_count());
    let mut scopes = Vec::new();
    if !quota.scopes.is_empty() {
        let joined: Vec<&str> = quota.scopes.iter().map(|s| &**s).collect();
        scopes.push(Row::pair("Scopes", joined.join(", ")));
    }
    scopes.extend(
        quota
            .scope_selector
            .iter()
            .map(|expr| Row::pair("Scope selector", expr.clone())),
    );
    let mut panel = Panel::new()
        .section("Resources", resources.into_iter().map(bar).collect())
        .section("Object counts", counts.into_iter().map(bar).collect())
        .section("Scopes", scopes);
    if panel.is_empty() {
        panel = panel.section("Usage", vec![Row::Note("No limits set.".to_owned())]);
    }
    Some(panel)
}

/// A LimitRange's limits: one table per limited type.
pub(super) fn limit_range(res: &Resource) -> Option<Panel> {
    let limits = LimitRangeSummary::from_resource(res).ok()?;
    let cell = |v: &Option<std::sync::Arc<str>>| v.as_deref().unwrap_or("-").to_owned();
    let mut panel = Panel::new();
    for (ix, section) in limits.sections.iter().enumerate() {
        let mut rows = vec![Row::head([
            "Resource",
            "Min",
            "Max",
            "Default",
            "Default request",
            "Max ratio",
        ])];
        rows.extend(section.rows.iter().map(|r| {
            Row::cells([
                r.resource.to_string(),
                cell(&r.min),
                cell(&r.max),
                cell(&r.default),
                cell(&r.default_request),
                cell(&r.max_ratio),
            ])
        }));
        // Two limits of the same type stay apart by their position.
        let title = if limits
            .sections
            .iter()
            .filter(|s| s.kind == section.kind)
            .count()
            > 1
        {
            format!("{} limits ({})", section.kind, ix + 1)
        } else {
            format!("{} limits", section.kind)
        };
        panel = panel.section(title, rows);
    }
    if panel.is_empty() {
        panel = panel.section("Limits", vec![Row::Note("No limits set.".to_owned())]);
    }
    Some(panel)
}
