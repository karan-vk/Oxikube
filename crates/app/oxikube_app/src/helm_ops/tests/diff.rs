//! The rollback preview's values diff.

use crate::helm_ops::{DiffKind, values_diff};

#[test]
fn equal_values_have_no_diff() {
    assert!(values_diff("a: 1\nb: 2\n", "a: 1\nb: 2\n").is_empty());
    assert!(values_diff("", "").is_empty());
}

#[test]
fn a_changed_value_shows_what_rolling_back_removes_and_brings_back() {
    let lines = values_diff(
        "replicas: 3\nimage: nginx:1.27\nmode: fast\n",
        "replicas: 3\nimage: nginx:1.25\nmode: fast\n",
    );
    let marks: Vec<_> = lines.iter().map(|l| (l.kind, l.text.as_str())).collect();
    assert_eq!(
        marks,
        [
            (DiffKind::Same, "replicas: 3"),
            (DiffKind::Removed, "image: nginx:1.27"),
            (DiffKind::Added, "image: nginx:1.25"),
            (DiffKind::Same, "mode: fast"),
        ]
    );
}

#[test]
fn distant_changes_are_separated_and_unchanged_stretches_dropped() {
    let now = (0..30).map(|i| format!("k{i}: {i}\n")).collect::<String>();
    let target = now.replace("k2: 2", "k2: 99").replace("k27: 27", "k27: 99");
    let lines = values_diff(&now, &target);
    assert!(lines.iter().any(|l| l.text == "..."));
    assert!(lines.len() < 15, "{} lines", lines.len());
}
