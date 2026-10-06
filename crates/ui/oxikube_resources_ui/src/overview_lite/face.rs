//! [`TileFace`]: what a tile says for a [`CountState`], as plain text and a tone. Pure, so every
//! state's wording is tested without a window.

use oxikube_app::CountState;
use oxikube_ui::tile::TileTone;

/// The text and tone of one tile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TileFace {
    /// The big value: the total, or a word or dash when there is no number.
    pub value: String,
    /// The line under it ("5 healthy").
    pub caption: String,
    /// The value's colour.
    pub tone: TileTone,
    /// The longer explanation shown on hover (the budget's or server's reason).
    pub hover: Option<String>,
}

/// The face of a tile whose kind is in `state`.
pub fn face(state: &CountState) -> TileFace {
    let plain = |value: &str, caption: &str, tone, hover: Option<&str>| TileFace {
        value: value.to_owned(),
        caption: caption.to_owned(),
        tone,
        hover: hover.filter(|h| !h.is_empty()).map(str::to_owned),
    };
    match state {
        CountState::Counted(count) if !count.has_health() => TileFace {
            value: count.total.to_string(),
            caption: String::new(),
            tone: TileTone::Neutral,
            hover: None,
        },
        CountState::Counted(count) => {
            let unhealthy = count.unhealthy();
            let tone = match (count.total, unhealthy) {
                (0, _) => TileTone::Neutral,
                (_, 0) => TileTone::Good,
                _ => TileTone::Warn,
            };
            let caption = if unhealthy == 0 {
                format!("{} healthy", count.healthy)
            } else {
                format!("{} healthy · {unhealthy} not", count.healthy)
            };
            TileFace {
                value: count.total.to_string(),
                caption,
                tone,
                hover: None,
            }
        }
        CountState::Loading => plain("…", "Loading", TileTone::Muted, None),
        CountState::NoAccess { message } => plain(
            "no access",
            "You may not list this kind",
            TileTone::Warn,
            Some(message),
        ),
        CountState::OverBudget { message } => plain(
            "–",
            "Not counted: over the watch budget",
            TileTone::Muted,
            Some(message),
        ),
        CountState::Failed { message } => {
            plain("–", "Could not count", TileTone::Muted, Some(message))
        }
        CountState::NotWatched => plain("–", "Not watched", TileTone::Muted, None),
    }
}
