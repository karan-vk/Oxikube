//! One script per windowed scenario. Each sets up through the user's commands (connect, open a
//! list, open a drawer), lets the first list land (the `setup` phase, reported apart), then runs
//! its scripted phases. A script fails, rather than measure something else, when its input does
//! not reach the view it is about (a scroll that does not scroll, a filter that does not filter).

mod catalog;
mod drawer;
mod logs;
mod table;
mod tabs;
mod terminal;

use std::f32::consts::TAU;
use std::time::Duration;

use anyhow::Result;
use gpui::{Pixels, Size, px, size};
use oxikube_domain::ids::Gvk;

use super::Scenario;
use super::driver::Driver;
use super::run::{WINDOW_SIZE, WindowRun};

/// After the first list lands: the table's first frames, ages and badges settle.
pub const SETTLE: Duration = Duration::from_secs(2);
/// Refreshes between two keystrokes: about 15 characters a second at 120 Hz, a fast typist.
pub const KEY_EVERY: u64 = 8;
/// One back-and-forth of a resize drag, in refreshes: two seconds at 120 Hz.
const RESIZE_PERIOD: f32 = 240.0;
/// The first pod of the main synthetic cluster.
pub const FIRST_POD: &str = "load-00000";

/// The `Pod` kind.
pub fn pods() -> Gvk {
    Gvk::new("", "v1", "Pod")
}

/// Where in its back-and-forth (radians) a resize drag is on refresh `index`.
pub fn drag_phase(index: u64) -> f32 {
    (index as f32 / RESIZE_PERIOD) * TAU
}

/// The window's size on refresh `index` of a continuous corner drag around [`WINDOW_SIZE`].
pub fn drag_size(index: u64) -> Size<Pixels> {
    let (width, height) = WINDOW_SIZE;
    let phase = drag_phase(index);
    size(
        px(width - 300.0 + 300.0 * phase.cos()),
        px(height - 150.0 + 150.0 * phase.sin()),
    )
}

/// The size every scenario starts at, restored after a drag.
pub fn window_size() -> Size<Pixels> {
    size(px(WINDOW_SIZE.0), px(WINDOW_SIZE.1))
}

/// Runs `run`'s script on `driver`.
pub async fn run(run: &WindowRun, driver: &mut Driver<'_>) -> Result<()> {
    match run.scenario {
        Scenario::PodsTable => table::pods_table(driver).await,
        Scenario::TableFilter => table::filter(driver).await,
        Scenario::Namespaces => table::namespaces(driver).await,
        Scenario::Theme => table::theme(driver).await,
        Scenario::Sidebar => table::sidebar(driver).await,
        Scenario::Idle => table::idle(driver).await,
        Scenario::DetailDrawer => drawer::run(driver).await,
        Scenario::TabsPanes => tabs::run(driver).await,
        Scenario::Catalog => catalog::run(driver).await,
        Scenario::Logs => logs::run(driver).await,
        Scenario::Terminal => terminal::run(driver, run.exec.as_ref()).await,
    }
}

/// A keystroke script: `text` typed one character every `every` refreshes, then erased with
/// backspace at the same pace, over and over. Returns the key to press on refresh `index`, if any.
pub fn typing(text: &str, every: u64, index: u64) -> Option<String> {
    if every == 0 || !index.is_multiple_of(every) {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len() as u64;
    if n == 0 {
        return None;
    }
    let k = (index / every) % (2 * n);
    Some(if k < n {
        key_for(chars[usize::try_from(k).unwrap_or(0)])
    } else {
        "backspace".to_owned()
    })
}

/// The keystroke that types `c`.
fn key_for(c: char) -> String {
    match c {
        ' ' => "space".to_owned(),
        '-' => "-".to_owned(),
        c if c.is_ascii_uppercase() => format!("shift-{}", c.to_ascii_lowercase()),
        c => c.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_types_then_erases_at_its_pace() {
        let keys: Vec<Option<String>> = (0..12).map(|i| typing("ab", 2, i)).collect();
        assert_eq!(
            keys,
            [
                Some("a".into()),
                None,
                Some("b".into()),
                None,
                Some("backspace".into()),
                None,
                Some("backspace".into()),
                None,
                Some("a".into()),
                None,
                Some("b".into()),
                None,
            ]
        );
        assert_eq!(typing("A x", 1, 0).as_deref(), Some("shift-a"));
        assert_eq!(typing("A x", 1, 1).as_deref(), Some("space"));
        assert_eq!(typing("", 1, 0), None);
    }
}
