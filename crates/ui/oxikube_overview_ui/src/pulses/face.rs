//! [`TileFace`]: what a tile says, as plain text, a level and a gauge. Pure functions of the
//! store's [`CountState`] or the cluster's [`ClusterFigure`], so every state's wording is tested
//! without a window.
//!
//! Colour is never the only signal (accessibility): a [`Level`] other than neutral carries a
//! [`status`](TileFace::status) word and an icon next to the title, and a gauge has its
//! percentage as text.

use oxikube_app::columns::Tone;
use oxikube_app::columns::UsageThresholds;
use oxikube_app::{CountState, FetchedCount, KindCount};
use oxikube_ui::IconName;

use super::specs::Resource;

/// How a tile reads, from calm to alarming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Level {
    /// A count with nothing to judge.
    #[default]
    Neutral,
    /// Everything is fine.
    Good,
    /// Something needs attention.
    Warn,
    /// Most of it is failing, or a usage is past its critical threshold.
    Critical,
    /// Not a number: loading, not counted, no access.
    Muted,
}

impl Level {
    /// The word shown next to the title, for the levels that colour alone would not carry.
    pub fn status(self) -> Option<&'static str> {
        match self {
            Level::Good => Some("OK"),
            Level::Warn => Some("Attention"),
            Level::Critical => Some("Critical"),
            Level::Neutral | Level::Muted => None,
        }
    }

    /// The icon shown next to the title with [`status`](Self::status).
    pub fn icon(self) -> Option<IconName> {
        match self {
            Level::Good => Some(IconName::CircleCheck),
            Level::Warn => Some(IconName::TriangleAlert),
            Level::Critical => Some(IconName::CircleX),
            Level::Neutral | Level::Muted => None,
        }
    }
}

/// A gauge: a bar filled to `fraction`, with its text.
#[derive(Clone, Debug, PartialEq)]
pub struct GaugeFace {
    /// How full, `0.0..=1.0`.
    pub fraction: f32,
    /// What the bar says in words (`92% healthy`, `40% of CPU`).
    pub text: String,
}

/// The text and shape of one tile.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TileFace {
    /// The big value: a count, a percent, or a dash or word when there is no number.
    pub value: String,
    /// The line under it (`12 ready · 2 not ready`).
    pub caption: String,
    /// How the tile reads.
    pub level: Level,
    /// The gauge, when the tile has something to measure.
    pub gauge: Option<GaugeFace>,
    /// The longer explanation shown on hover (the server's or the budget's reason).
    pub hover: Option<String>,
}

/// Whether the one-off count behind a tile whose kind has no feed is there.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Fallback {
    /// Not asked for (the kind has a feed, or nothing needs one).
    #[default]
    None,
    /// The request is out.
    Loading,
    /// The server's answer.
    Counted(FetchedCount),
    /// The request failed.
    Failed(String),
}

/// The words for the two sides of a kind's health.
fn health_words(plural: &str) -> (&'static str, &'static str) {
    match plural {
        "cronjobs" => ("active", "suspended"),
        "jobs" => ("ok", "failed"),
        "pods" => ("healthy", "unhealthy"),
        _ => ("ready", "not ready"),
    }
}

/// The face of the kind `plural` in `state`; `fallback` is the one-off count behind a kind with
/// no feed.
pub fn kind_face(plural: &str, state: &CountState, fallback: &Fallback) -> TileFace {
    match state {
        CountState::Counted(count) => counted_face(plural, *count),
        CountState::Loading => plain("…", "Loading", Level::Muted, None),
        CountState::NoAccess { message } => plain(
            "no access",
            "You may not list this kind",
            Level::Warn,
            Some(message),
        ),
        CountState::OverBudget { message } => fallback_face(
            fallback,
            "Over the watch budget",
            message,
            "Not counted: over the watch budget",
        ),
        CountState::Failed { message } => {
            fallback_face(fallback, "Feed failed", message, "Could not count")
        }
        CountState::NotWatched => fallback_face(fallback, "Not watched", "", "Not watched"),
    }
}

fn counted_face(plural: &str, count: KindCount) -> TileFace {
    if !count.has_health() {
        return TileFace {
            value: count.total.to_string(),
            ..TileFace::default()
        };
    }
    let (good, bad) = health_words(plural);
    let unhealthy = count.unhealthy();
    let level = match (count.total, unhealthy) {
        (0, _) => Level::Neutral,
        (_, 0) => Level::Good,
        // Half or more of what there is, not working: past "attention".
        _ if unhealthy * 2 >= count.rated => Level::Critical,
        _ => Level::Warn,
    };
    let caption = if unhealthy == 0 {
        format!("{} {good}", count.healthy)
    } else {
        format!("{} {good} · {unhealthy} {bad}", count.healthy)
    };
    let fraction = count.healthy as f32 / count.rated as f32;
    TileFace {
        value: count.total.to_string(),
        caption,
        level,
        gauge: Some(GaugeFace {
            fraction,
            text: format!("{}% {good}", percent_text(f64::from(fraction) * 100.0)),
        }),
        hover: None,
    }
}

/// A kind with no feed: the one-off count when there is one, else a dash and why.
fn fallback_face(fallback: &Fallback, loading: &str, hover: &str, dash_caption: &str) -> TileFace {
    let hover = (!hover.is_empty()).then(|| hover.to_owned());
    match fallback {
        Fallback::Counted(count) => TileFace {
            value: if count.exact {
                count.total.to_string()
            } else {
                format!("≥{}", count.total)
            },
            caption: "Counted once · r to refresh".to_owned(),
            level: Level::Muted,
            gauge: None,
            hover,
        },
        Fallback::Loading => TileFace {
            value: "…".to_owned(),
            caption: format!("{loading} · counting"),
            level: Level::Muted,
            gauge: None,
            hover,
        },
        Fallback::Failed(message) => dash(
            "Could not count · r to retry".to_owned(),
            Some(message.clone()),
        ),
        Fallback::None => dash(dash_caption.to_owned(), hover),
    }
}

/// A muted dash with `caption` under it: the tile has no number to show.
fn dash(caption: String, hover: Option<String>) -> TileFace {
    TileFace {
        value: "–".to_owned(),
        caption,
        level: Level::Muted,
        gauge: None,
        hover,
    }
}

fn plain(value: &str, caption: &str, level: Level, hover: Option<&str>) -> TileFace {
    TileFace {
        value: value.to_owned(),
        caption: caption.to_owned(),
        level,
        gauge: None,
        hover: hover.filter(|h| !h.is_empty()).map(str::to_owned),
    }
}

/// What the page knows of the cluster's CPU or memory.
#[derive(Clone, Debug, PartialEq)]
pub enum ClusterFigure {
    /// Metrics were asked for; nothing came back yet.
    Waiting,
    /// There are no metrics: `metrics unavailable: <reason>`, never a zero.
    Unavailable(String),
    /// The nodes' readings against their allocatable.
    Reading {
        /// The latest readings summed (nanocores, bytes).
        used: u128,
        /// The reporting nodes' allocatable (nanocores, bytes).
        allocatable: u128,
        /// `used` of `allocatable` in percent; `None` when the nodes offer none.
        percent: Option<f64>,
        /// Nodes in the cluster.
        nodes: usize,
        /// Nodes with a reading.
        nodes_reporting: usize,
    },
}

/// The face of the cluster's `resource` gauge, toned by the cluster's `thresholds`.
pub fn cluster_face(
    resource: Resource,
    figure: &ClusterFigure,
    thresholds: &UsageThresholds,
) -> TileFace {
    match figure {
        ClusterFigure::Waiting => plain("…", "Waiting for metrics", Level::Muted, None),
        ClusterFigure::Unavailable(reason) => dash(reason.clone(), Some(reason.clone())),
        ClusterFigure::Reading {
            used,
            allocatable,
            percent,
            nodes,
            nodes_reporting,
        } => {
            let amounts = match resource {
                Resource::Cpu => format!("{} / {} cores", cores(*used), cores(*allocatable)),
                Resource::Memory => format!("{} / {}", bytes(*used), bytes(*allocatable)),
            };
            let caption = if nodes_reporting < nodes {
                format!("{amounts} · {nodes_reporting} of {nodes} nodes")
            } else {
                amounts
            };
            let Some(percent) = *percent else {
                return dash("Nodes report no allocatable".to_owned(), None);
            };
            let level = match thresholds.of(resource == Resource::Cpu).tone(percent) {
                Tone::Error => Level::Critical,
                Tone::Warn => Level::Warn,
                Tone::Neutral | Tone::Ok => Level::Good,
            };
            let shown = percent_text(percent);
            TileFace {
                value: format!("{shown}%"),
                caption,
                level,
                gauge: Some(GaugeFace {
                    fraction: (percent / 100.0).clamp(0.0, 1.0) as f32,
                    text: format!(
                        "{shown}% of {}",
                        match resource {
                            Resource::Cpu => "CPU",
                            Resource::Memory => "memory",
                        }
                    ),
                }),
                hover: None,
            }
        }
    }
}

/// A percent as shown: whole numbers, one decimal below 10 so a small figure is not `0`.
fn percent_text(percent: f64) -> String {
    if percent < 10.0 {
        trim(format!("{percent:.1}"))
    } else {
        format!("{percent:.0}")
    }
}

/// `1.5`, `8`: nanocores as cores, one decimal and no trailing `.0`.
fn cores(nanocores: u128) -> String {
    trim(format!("{:.1}", nanocores as f64 / 1e9))
}

/// `3 GiB`, `512 MiB`: bytes in the largest unit that keeps the number above one.
fn bytes(bytes: u128) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= 1024.0 * MIB {
        format!("{} GiB", trim(format!("{:.1}", value / (1024.0 * MIB))))
    } else {
        format!("{} MiB", trim(format!("{:.0}", value / MIB)))
    }
}

fn trim(text: String) -> String {
    match text.strip_suffix(".0") {
        Some(whole) => whole.to_owned(),
        None => text,
    }
}
