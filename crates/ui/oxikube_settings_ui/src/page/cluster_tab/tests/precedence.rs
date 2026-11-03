//! The precedence strip and the protection warnings.

use oxikube_settings::{LayerValues, SettingSource};
use serde_json::json;

use crate::page::cluster_tab::{Precedence, protection_warning};

fn winner(precedence: &Precedence) -> SettingSource {
    precedence
        .cells
        .iter()
        .find(|cell| cell.winner)
        .map(|cell| cell.layer)
        .expect("exactly one layer wins")
}

#[test]
fn a_default_only_field_is_won_by_the_default() {
    let values = LayerValues {
        default: Some(json!(false)),
        ..LayerValues::default()
    };
    let strip = Precedence::new(&values, SettingSource::Default);
    assert_eq!(winner(&strip), SettingSource::Default);
    assert!(!strip.overridden());
    assert_eq!(strip.cells[0].value.as_deref(), Some("false"));
    assert_eq!(strip.cells[1].value, None);
    assert_eq!(strip.cells[2].value, None);
}

#[test]
fn a_user_value_beats_the_default() {
    let values = LayerValues {
        default: Some(json!(false)),
        user: Some(json!(true)),
        cluster: None,
    };
    let strip = Precedence::new(&values, SettingSource::User);
    assert_eq!(winner(&strip), SettingSource::User);
    assert!(!strip.overridden());
    assert_eq!(strip.cells.iter().filter(|cell| cell.winner).count(), 1);
}

#[test]
fn a_cluster_value_beats_both() {
    let values = LayerValues {
        default: Some(json!(false)),
        user: Some(json!(true)),
        cluster: Some(json!(false)),
    };
    let strip = Precedence::new(&values, SettingSource::Cluster);
    assert_eq!(winner(&strip), SettingSource::Cluster);
    assert!(strip.overridden());
    assert!(strip.summary().contains("this cluster"));
}

#[test]
fn long_values_are_cut() {
    let long = json!("x".repeat(100));
    let strip = Precedence::new(
        &LayerValues {
            user: Some(long),
            ..LayerValues::default()
        },
        SettingSource::User,
    );
    let shown = strip.cells[1].value.as_deref().expect("a value");
    assert!(shown.chars().count() <= 29, "{shown}");
    assert!(shown.ends_with('\u{2026}'));
}

#[test]
fn weakening_overrides_warn_and_strengthening_ones_do_not() {
    let (on, off) = (json!(true), json!(false));
    // Read-only off here while on elsewhere: a warning. The other way round, or no change: none.
    assert!(protection_warning("read_only", Some(&on), Some(&off)).is_some());
    assert!(protection_warning("read_only", Some(&off), Some(&on)).is_none());
    assert!(protection_warning("read_only", None, Some(&off)).is_none());
    assert!(protection_warning("read_only", Some(&on), None).is_none());
    assert!(protection_warning("exec_in_read_only", None, Some(&on)).is_some());
    assert!(protection_warning("exec_in_read_only", Some(&on), Some(&on)).is_none());
    assert!(protection_warning("guard.default_read_only", Some(&on), Some(&off)).is_some());
    assert!(protection_warning("default_namespace", Some(&on), Some(&off)).is_none());
}
