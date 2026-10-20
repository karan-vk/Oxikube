use proptest::prelude::*;

use crate::portforward::annotations::{MAX_ANNOTATION_LEN, MAX_ENTRIES, parse_k9s_forwards};

fn fwd(
    container: Option<&str>,
    local: u16,
    remote: u16,
    auto: bool,
) -> super::super::SuggestedForward {
    super::super::SuggestedForward {
        container: container.map(str::to_owned),
        local_port: local,
        remote_port: remote,
        auto,
    }
}

#[test]
fn valid_entries_parse() {
    let cases: &[(&str, Vec<super::super::SuggestedForward>)] = &[
        ("web::8080:80", vec![fwd(Some("web"), 8080, 80, false)]),
        ("8080:80", vec![fwd(None, 8080, 80, false)]),
        ("web::9000", vec![fwd(Some("web"), 9000, 9000, false)]),
        ("9000", vec![fwd(None, 9000, 9000, false)]),
        ("::8080:80", vec![fwd(None, 8080, 80, false)]),
        (
            "  web :: 8080 : 80 ,\tdb::5432:5432 , ",
            vec![
                fwd(Some("web"), 8080, 80, false),
                fwd(Some("db"), 5432, 5432, false),
            ],
        ),
        (
            "web::8080:80,web::8080:80",
            vec![fwd(Some("web"), 8080, 80, false)],
        ),
        ("", vec![]),
        (" , ,", vec![]),
    ];
    for (text, want) in cases {
        let parsed = parse_k9s_forwards(text, false);
        assert_eq!(&parsed.forwards, want, "{text:?}");
        assert!(parsed.issues.is_empty(), "{text:?}: {:?}", parsed.issues);
    }
}

#[test]
fn the_auto_flag_follows_the_annotation() {
    let parsed = parse_k9s_forwards("web::8080:80", true);
    assert_eq!(parsed.forwards, vec![fwd(Some("web"), 8080, 80, true)]);
}

#[test]
fn a_bad_entry_is_skipped_and_reported_without_losing_the_rest() {
    for bad in [
        "web::0:80",
        "web::8080:0",
        "web::70000:80",
        "web::99999999999999999999:80",
        "web::abc",
        "web::8080:",
        "web::-1:80",
        "we b::8080:80",
        "web/x::8080:80",
        "8080:80:90",
        "web::",
        "+80",
    ] {
        let parsed = parse_k9s_forwards(&format!("{bad}, db::5432:5432"), false);
        assert_eq!(
            parsed.forwards,
            vec![fwd(Some("db"), 5432, 5432, false)],
            "{bad}"
        );
        assert_eq!(parsed.issues.len(), 1, "{bad}: {:?}", parsed.issues);
    }
}

#[test]
fn control_characters_never_reach_the_issue_text() {
    let parsed = parse_k9s_forwards("we\u{1b}[31mb::x\u{7}\n,ok::1:2", false);
    let issue = &parsed.issues[0];
    assert!(!issue.entry.chars().any(char::is_control), "{issue:?}");
    assert!(!issue.reason.chars().any(char::is_control), "{issue:?}");
}

#[test]
fn huge_input_is_bounded() {
    let entry_count = MAX_ENTRIES + 10;
    let many = (1..=entry_count)
        .map(|i| format!("{}:{}", 1000 + i, i))
        .collect::<Vec<_>>()
        .join(",");
    let parsed = parse_k9s_forwards(&many, false);
    assert_eq!(parsed.forwards.len(), MAX_ENTRIES);
    assert_eq!(parsed.issues.len(), 1);

    let long = "a".repeat(MAX_ANNOTATION_LEN * 4);
    let parsed = parse_k9s_forwards(&long, false);
    assert!(parsed.forwards.is_empty());
    assert!(parsed.issues.iter().all(|i| i.entry.chars().count() <= 49));
    let multibyte = "é".repeat(MAX_ANNOTATION_LEN);
    parse_k9s_forwards(&multibyte, false);
}

proptest! {
    #[test]
    fn never_panics(text in any::<String>(), auto in any::<bool>()) {
        let parsed = parse_k9s_forwards(&text, auto);
        prop_assert!(parsed.forwards.len() <= MAX_ENTRIES);
        for f in &parsed.forwards {
            prop_assert!(f.local_port != 0 && f.remote_port != 0);
        }
        for i in &parsed.issues {
            prop_assert!(!i.entry.chars().any(char::is_control));
        }
    }

    #[test]
    fn a_written_entry_round_trips(local in 1u16..=u16::MAX, remote in 1u16..=u16::MAX, name in "[a-z][a-z0-9-]{0,20}") {
        let parsed = parse_k9s_forwards(&format!("{name}::{local}:{remote}"), false);
        prop_assert_eq!(parsed.forwards, vec![fwd(Some(&name), local, remote, false)]);
    }
}
