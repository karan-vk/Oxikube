//! The status bar of a tile: its segments as plain data, and the colour of each bucket. Pure, so
//! the proportions and the tones are tested without a window.

use oxikube_app::KindCount;
use oxikube_domain::view::{Bucket, kind_buckets};
use oxikube_ui::Colors;

use gpui::Hsla;

/// One coloured stretch of a tile's bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    /// The status bucket it stands for; a click filters the table to it.
    pub bucket: Bucket,
    /// How many objects are in it.
    pub count: usize,
    /// Its share of the bar, 0 to 1 (the segments sum to 1).
    pub fraction: f32,
}

/// The segments of the kind `group`/`kind` for `count`, in the order the bar draws them, skipping
/// empty buckets. Empty when the kind has no buckets or nothing is rated.
pub fn segments(group: &str, kind: &str, count: &KindCount) -> Vec<Segment> {
    let total: usize = kind_buckets(group, kind)
        .iter()
        .map(|b| count.buckets.get(*b))
        .sum();
    if total == 0 {
        return Vec::new();
    }
    count
        .buckets
        .in_order(kind_buckets(group, kind))
        .map(|(bucket, n)| Segment {
            bucket,
            count: n,
            fraction: n as f32 / total as f32,
        })
        .collect()
}

/// The theme colour of `bucket`: the status tokens, so a theme change recolours the bars.
pub fn colour(bucket: Bucket, colors: &Colors) -> Hsla {
    match bucket {
        Bucket::Running | Bucket::Ready | Bucket::Active => colors.success,
        Bucket::Succeeded => colors.info,
        Bucket::NotReady | Bucket::Pending => colors.warning,
        Bucket::Failed => colors.error,
        Bucket::Unknown | Bucket::Suspended => colors.text_disabled,
    }
}

/// The hover text of a segment: what it counts and what a click does.
pub fn hover(segment: &Segment) -> String {
    format!(
        "{} {}: click to list them",
        segment.count,
        segment.bucket.label().to_lowercase()
    )
}
