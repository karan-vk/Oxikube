//! Names, labels and the manifest.

use std::collections::BTreeMap;

use crate::namespaces::{LabelError, manifest, parse_labels, validate_labels, validate_name};

#[test]
fn names_are_dns_labels() {
    for ok in ["a", "team-a", "a1", "0day", &"x".repeat(63)] {
        assert!(validate_name(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "Team",
        "team_a",
        "-a",
        "a-",
        "a.b",
        "a b",
        &"x".repeat(64),
        "é",
    ] {
        assert!(validate_name(bad).is_err(), "{bad:?}");
    }
    let message = validate_name("Team").unwrap_err().to_string();
    assert!(
        message.contains("lowercase"),
        "the rule is spelled out: {message}"
    );
}

#[test]
fn labels_parse_from_commas_or_spaces() {
    assert!(parse_labels("").unwrap().is_empty());
    assert!(parse_labels("  ,, ").unwrap().is_empty());
    let labels = parse_labels("team=a, env=dev  tier=").unwrap();
    assert_eq!(
        labels,
        BTreeMap::from([
            ("env".to_owned(), "dev".to_owned()),
            ("team".to_owned(), "a".to_owned()),
            ("tier".to_owned(), String::new()),
        ])
    );
    let labels = parse_labels("example.com/owner=platform").unwrap();
    assert_eq!(labels["example.com/owner"], "platform");
}

#[test]
fn bad_labels_say_which() {
    assert_eq!(
        parse_labels("team"),
        Err(LabelError::NotKeyValue("team".into()))
    );
    assert_eq!(parse_labels("=a"), Err(LabelError::Key(String::new())));
    assert_eq!(
        parse_labels("a b=c").unwrap_err(),
        LabelError::NotKeyValue("a".into())
    );
    assert!(matches!(parse_labels("-x=1"), Err(LabelError::Key(_))));
    assert!(matches!(parse_labels("UP/x=1"), Err(LabelError::Key(_))));
    assert!(matches!(
        parse_labels("team=-a"),
        Err(LabelError::Value { .. })
    ));
    let long = "v".repeat(64);
    assert!(validate_labels(&BTreeMap::from([("k".to_owned(), long)])).is_err());
}

#[test]
fn the_manifest_carries_the_name_and_labels() {
    let labels = BTreeMap::from([("team".to_owned(), "a".to_owned())]);
    assert_eq!(
        manifest("team-a", &labels),
        serde_json::json!({
            "apiVersion": "v1",
            "kind": "Namespace",
            "metadata": { "name": "team-a", "labels": { "team": "a" } },
        })
    );
    assert!(
        manifest("x", &BTreeMap::new())["metadata"]
            .get("labels")
            .is_none()
    );
}
