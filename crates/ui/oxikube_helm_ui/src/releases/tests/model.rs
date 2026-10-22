//! The model's wording: tabs, tones, ages, links.

use oxikube_domain::helm::ReleaseStatus;

use crate::releases::{Tab, Tone, ValuesMode, age, links, tone};

#[test]
fn tabs_cycle_in_display_order() {
    let titles: Vec<_> = Tab::ALL.iter().map(|t| t.title()).collect();
    assert_eq!(
        titles,
        ["Resources", "Values", "Manifest", "Notes", "History"]
    );
    assert_eq!(Tab::History.next(), Tab::Resources);
    assert_eq!(Tab::Resources.previous(), Tab::History);
    assert_eq!(Tab::Values.next(), Tab::Manifest);
}

#[test]
fn every_status_has_a_tone() {
    for status in ReleaseStatus::ALL {
        let _ = tone(status);
    }
    assert_eq!(tone(ReleaseStatus::Deployed), Tone::Good);
    assert_eq!(tone(ReleaseStatus::Failed), Tone::Bad);
    assert_eq!(tone(ReleaseStatus::PendingUpgrade), Tone::Busy);
    assert_eq!(tone(ReleaseStatus::Superseded), Tone::Quiet);
    assert_eq!(tone(ReleaseStatus::Uninstalled), Tone::Quiet);
}

#[test]
fn ages_read_like_the_resource_tables() {
    let now = super::at(1_000_000);
    let ago = |secs: i64| age(super::at(1_000_000 - secs), now);
    assert_eq!(ago(0), "0s");
    assert_eq!(ago(59), "59s");
    assert_eq!(ago(60), "1m");
    assert_eq!(ago(3_599), "59m");
    assert_eq!(ago(3_600), "1h");
    assert_eq!(ago(86_400 * 3), "3d");
    assert_eq!(
        age(super::at(1_000_100), now),
        "0s",
        "a future stamp is not negative"
    );
}

#[test]
fn values_mode_toggles() {
    assert!(!ValuesMode::User.all());
    assert_eq!(ValuesMode::User.toggled(), ValuesMode::All);
    assert_eq!(ValuesMode::All.toggled(), ValuesMode::User);
}

#[test]
fn notes_links_are_web_links_only_without_repeats() {
    let notes = "Open http://localhost:8080/ or (https://example.com/docs). \
                 Again https://example.com/docs, and file:///etc/passwd and javascript:alert(1)";
    assert_eq!(
        links(notes, 8),
        ["http://localhost:8080/", "https://example.com/docs"]
    );
    assert_eq!(links(notes, 1).len(), 1);
    assert!(links("no links here", 8).is_empty());
}
