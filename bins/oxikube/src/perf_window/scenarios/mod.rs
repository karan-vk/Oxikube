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

use std::time::Duration;

use anyhow::Result;

use super::Scenario;
use super::driver::Driver;
use super::run::WindowRun;

/// After the first list lands: the table's first frames, ages and badges settle.
pub const SETTLE: Duration = Duration::from_secs(2);

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
