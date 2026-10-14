//! The list on a manual clock: dedupe, grouping of a burst, read state, redaction and the bound.

use super::{at, cluster, grouped, input, other, secs};
use crate::notifications::NotificationRules;
use crate::notifications::centre::Centre;
use crate::notifications::limiter::Limiter;

struct Fixture {
    centre: Centre,
    limiter: Limiter,
    rules: NotificationRules,
}

impl Fixture {
    fn new() -> Self {
        Self::with(NotificationRules::default())
    }

    fn with(rules: NotificationRules) -> Self {
        Self {
            centre: Centre::default(),
            limiter: Limiter::default(),
            rules,
        }
    }

    fn apply(
        &mut self,
        input: crate::notifications::NotificationInput,
        secs: i64,
    ) -> super::super::centre::Applied {
        self.centre
            .apply(input, &mut self.limiter, &self.rules, at(secs))
    }

    fn titles(&self) -> Vec<String> {
        self.centre.list().iter().map(|n| n.title.clone()).collect()
    }
}

#[test]
fn the_same_key_within_the_window_counts_up_instead_of_adding_a_row() {
    let mut fx = Fixture::new();
    let first = fx.apply(input(&cluster(), "k"), 0);
    let mut more = input(&cluster(), "k");
    more.occurrences = 4;
    more.body = "the latest body".into();
    let second = fx.apply(more, 30);
    assert_eq!(first.row.id, second.row.id);
    assert_eq!(fx.centre.len(), 1);
    assert_eq!(second.row.count, 5);
    assert_eq!(second.row.body, "the latest body");
    assert_eq!(second.row.created, at(0));
    assert_eq!(second.row.updated, at(30));
}

#[test]
fn the_same_key_after_the_dedupe_window_is_a_new_row() {
    let mut fx = Fixture::new();
    let window = fx.rules.dedupe_window.as_secs() as i64;
    let first = fx.apply(input(&cluster(), "k"), 0);
    let later = fx.apply(input(&cluster(), "k"), window + 1);
    assert_ne!(first.row.id, later.row.id);
    assert_eq!(fx.centre.len(), 2);
    // And the key now names the new row.
    let again = fx.apply(input(&cluster(), "k"), window + 2);
    assert_eq!(again.row.id, later.row.id);
}

#[test]
fn a_burst_of_one_reason_folds_into_one_group_row() {
    let mut fx = Fixture::new();
    for (i, object) in ["a", "b", "c", "d", "e", "f"].iter().enumerate() {
        fx.apply(grouped(&cluster(), object, "BackOff"), i as i64);
    }
    // Three objects get rows of their own, the next three fold into "3 pods: BackOff".
    assert_eq!(
        fx.titles(),
        [
            "3 pods: BackOff",
            "Pod default/c: BackOff",
            "Pod default/b: BackOff",
            "Pod default/a: BackOff",
        ]
    );
    let group = &fx.centre.list()[0];
    assert!(group.grouped);
    assert_eq!(group.objects, 3);
    assert_eq!(group.count, 3);
    assert!(
        group.body.starts_with("Latest: Pod default/f: BackOff."),
        "{}",
        group.body
    );
    // The same object again within the burst counts, but is no new object.
    fx.apply(grouped(&cluster(), "f", "BackOff"), 7);
    let group = &fx.centre.list()[0];
    assert_eq!((group.objects, group.count), (3, 4));
}

#[test]
fn bursts_are_counted_per_cluster_and_per_reason() {
    let mut fx = Fixture::new();
    for (i, object) in ["a", "b", "c"].iter().enumerate() {
        fx.apply(grouped(&cluster(), object, "BackOff"), i as i64);
        fx.apply(grouped(&cluster(), object, "Failed"), i as i64);
        fx.apply(grouped(&other(), object, "BackOff"), i as i64);
    }
    assert_eq!(fx.centre.len(), 9, "nobody reached the threshold twice");
    assert!(fx.centre.list().iter().all(|n| !n.grouped));
}

#[test]
fn a_burst_ends_with_its_window() {
    let mut fx = Fixture::new();
    let window = fx.rules.group_window.as_secs() as i64;
    for (i, object) in ["a", "b", "c"].iter().enumerate() {
        fx.apply(grouped(&cluster(), object, "BackOff"), i as i64);
    }
    let late = fx.apply(grouped(&cluster(), "d", "BackOff"), window + 10);
    assert!(!late.row.grouped, "the earlier objects left the window");
}

#[test]
fn mixed_kinds_in_a_group_are_objects() {
    let mut fx = Fixture::new();
    for object in ["a", "b", "c", "d"] {
        fx.apply(grouped(&cluster(), object, "FailedMount"), 0);
    }
    let mut node = grouped(&cluster(), "n1", "FailedMount");
    node.group.as_mut().unwrap().noun = "node".into();
    let row = fx.apply(node, 1).row;
    assert_eq!(row.title, "2 objects: FailedMount");
}

#[test]
fn rows_are_unread_until_read_and_a_new_occurrence_makes_them_unread_again() {
    let mut fx = Fixture::new();
    let a = fx.apply(input(&cluster(), "a"), 0).row.id;
    let b = fx.apply(input(&cluster(), "b"), 1).row.id;
    assert_eq!(fx.centre.unread(), 2);
    assert!(fx.centre.mark_read(a));
    assert!(!fx.centre.mark_read(a), "already read");
    assert_eq!(fx.centre.unread(), 1);
    assert!(fx.centre.get(a).unwrap().read);
    fx.apply(input(&cluster(), "a"), 2);
    assert_eq!(fx.centre.unread(), 2);
    assert!(!fx.centre.get(a).unwrap().read);
    assert_eq!(fx.centre.mark_all_read(), 2);
    assert_eq!(fx.centre.unread(), 0);
    assert!(fx.centre.get(b).unwrap().read);
    assert_eq!(fx.centre.mark_all_read(), 0);
}

#[test]
fn the_newest_occurrence_lists_first() {
    let mut fx = Fixture::new();
    fx.apply(input(&cluster(), "a"), 0);
    fx.apply(input(&cluster(), "b"), 1);
    fx.apply(input(&cluster(), "a"), 2);
    assert_eq!(fx.titles(), ["title a", "title b"]);
}

#[test]
fn the_oldest_rows_go_past_the_bound() {
    let mut fx = Fixture::with(NotificationRules {
        capacity: 3,
        ..NotificationRules::default()
    });
    for key in ["a", "b", "c", "d", "e"] {
        fx.apply(input(&cluster(), key), 0);
    }
    assert_eq!(fx.titles(), ["title e", "title d", "title c"]);
    assert_eq!(fx.centre.unread(), 3);
    // An evicted key starts a new row.
    let again = fx.apply(input(&cluster(), "a"), 1).row;
    assert_eq!(again.count, 1);
}

#[test]
fn text_is_redacted_flattened_and_capped() {
    let mut fx = Fixture::new();
    let mut secret = input(&cluster(), "k");
    secret.title = "token=abcdef1234567890abcdef\nsecond line".into();
    secret.body = "x".repeat(2_000);
    let row = fx.apply(secret, 0).row;
    assert!(
        !row.title.contains("abcdef1234567890abcdef"),
        "{}",
        row.title
    );
    assert!(!row.title.contains('\n'));
    assert_eq!(
        row.body.chars().count(),
        crate::notifications::MAX_BODY_CHARS
    );
    assert!(row.body.ends_with('\u{2026}'));
}

#[test]
fn a_row_raises_once_per_object_interval() {
    let mut fx = Fixture::new();
    let interval = fx.rules.object_interval.as_secs() as i64;
    assert!(fx.apply(input(&cluster(), "k"), 0).raised, "new: toast");
    assert!(
        !fx.apply(input(&cluster(), "k"), 10).raised,
        "again soon: list only"
    );
    assert!(
        fx.apply(input(&cluster(), "k"), interval + 1).raised,
        "later: toast again"
    );
    let _ = secs(0);
}
