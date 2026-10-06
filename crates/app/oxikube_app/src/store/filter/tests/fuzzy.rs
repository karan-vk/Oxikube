//! Fuzzy matching and ranking.

use crate::store::filter::Fuzzy;

fn rank(query: &str, names: &[&str]) -> Vec<String> {
    let fuzzy = Fuzzy::new(query);
    let mut scored: Vec<(i32, &str)> = names
        .iter()
        .filter_map(|n| fuzzy.score(n).map(|s| (s, *n)))
        .collect();
    // Best first, ties by name: the order the store gives (it breaks ties on the object key).
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    scored.into_iter().map(|(_, n)| n.to_owned()).collect()
}

#[test]
fn matches_are_subsequences() {
    let f = Fuzzy::new("wbp");
    assert!(f.matches("web-pod"));
    assert!(f.matches("WEB-POD"));
    assert!(!f.matches("pod-web"));
    assert!(!f.matches("wb"));
    assert!(Fuzzy::new("").matches("anything"));
    assert!(Fuzzy::new("  ").is_empty(), "whitespace is dropped");
    assert_eq!(f.score("pod-web"), None);
    assert!(f.score("web-pod").is_some());
}

#[test]
fn consecutive_and_word_start_matches_rank_first() {
    let names = ["a-x-b-x-c", "abc-pod", "xaxbxc", "a-b-c"];
    assert_eq!(
        rank("abc", &names),
        ["abc-pod", "a-b-c", "a-x-b-x-c", "xaxbxc"]
    );
    assert_eq!(
        rank("web", &["my-web", "webapp", "w-e-b", "awebz"]),
        ["webapp", "my-web", "awebz", "w-e-b"]
    );
}

#[test]
fn the_best_alignment_wins_not_the_first() {
    let f = Fuzzy::new("ab");
    // The leftmost greedy match would take the first `a`; the best alignment is the `ab` pair.
    assert!(f.score("a_xxxxab").unwrap() > f.score("a_xxxxxxxb").unwrap());
    assert!(f.score("ab").unwrap() > f.score("a_b").unwrap());
}

#[test]
fn ranking_is_deterministic() {
    let names = ["pod-b", "pod-a", "pod-c", "xpodx"];
    let first = rank("pod", &names);
    for _ in 0..5 {
        assert_eq!(rank("pod", &names), first);
    }
    assert_eq!(first, ["pod-a", "pod-b", "pod-c", "xpodx"]);
}

#[test]
fn adding_characters_narrows() {
    let (short, long) = (Fuzzy::new("wb"), Fuzzy::new("wbp"));
    assert!(long.narrows(&short));
    assert!(!short.narrows(&long));
    assert!(short.narrows(&short));
    assert!(
        Fuzzy::new("wxb").narrows(&short),
        "inserting in the middle narrows too"
    );
    assert!(!Fuzzy::new("bw").narrows(&short));
}
