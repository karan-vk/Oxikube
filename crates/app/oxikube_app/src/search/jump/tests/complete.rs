//! Completion: which word is being typed, what it can be, and what accepting one does.

use super::super::{Slot, accept, candidates, site};
use super::Env;

fn slot(line: &str) -> Slot {
    site(line).slot
}

#[test]
fn the_first_word_is_an_alias() {
    for line in ["", "p", "pod", ":", ":dep"] {
        assert_eq!(slot(line), Slot::Alias, "{line:?}");
    }
    assert_eq!(site(":dep").prefix, "dep");
}

#[test]
fn the_second_word_is_a_namespace() {
    assert_eq!(slot("deploy "), Slot::Namespace);
    let s = site("deploy kube-");
    assert_eq!((s.slot, s.prefix.as_str()), (Slot::Namespace, "kube-"));
    assert_eq!(slot("ns "), Slot::Namespace);
}

#[test]
fn a_word_after_an_at_sign_or_ctx_is_a_context() {
    let s = site("pods @pr");
    assert_eq!((s.slot, s.prefix.as_str()), (Slot::Context, "pr"));
    assert_eq!(&"pods @pr"[s.replace.range()], "pr", "the @ stays");
    assert_eq!(slot("pods kube-system @"), Slot::Context);
    assert_eq!(slot("ctx "), Slot::Context);
    assert_eq!(slot("ctx pr"), Slot::Context);
}

#[test]
fn filters_selectors_and_arguments_of_the_other_words_have_nothing_to_complete() {
    for line in [
        "pods /ap",
        "pods app=",
        "pods /-l ",
        "pods /-l ap",
        "q ",
        "ctx prod ",
        "ns web ",
        "- ",
    ] {
        assert_eq!(slot(line), Slot::Nothing, "{line:?}");
    }
}

#[test]
fn a_second_namespace_is_not_offered() {
    assert_eq!(slot("pods web "), Slot::Nothing);
    assert_eq!(slot("pods /api web x"), Slot::Nothing);
    // The operand of a flag is not a namespace, so one can still follow it.
    assert_eq!(slot("pods /-l a=b "), Slot::Namespace);
}

#[test]
fn aliases_are_the_clusters_names_plus_the_reserved_words() {
    let env = Env::new();
    let names: Vec<String> = candidates(Slot::Alias, &env)
        .iter()
        .map(|c| c.text.to_string())
        .collect();
    for word in [
        "pods",
        "po",
        "deploy",
        "ctx",
        "ns",
        "q",
        "certs",
        "certificates.cert-manager.io",
    ] {
        assert!(names.iter().any(|n| n == word), "{word}");
    }
    assert!(names.windows(2).all(|w| w[0] < w[1]), "sorted and unique");
    let deploy = candidates(Slot::Alias, &env)
        .into_iter()
        .find(|c| &*c.text == "deploy")
        .unwrap();
    assert!(deploy.detail.contains("deployments"), "{}", deploy.detail);
}

#[test]
fn a_cluster_without_crds_offers_none() {
    let mut env = Env::new();
    env.active = Some(super::cluster("prod-eu"));
    let names: Vec<_> = candidates(Slot::Alias, &env)
        .iter()
        .map(|c| c.text.to_string())
        .collect();
    assert!(!names.iter().any(|n| n == "certs"));
}

#[test]
fn namespaces_come_from_the_clusters_list_with_all_first() {
    let env = Env::new();
    let names: Vec<_> = candidates(Slot::Namespace, &env)
        .iter()
        .map(|c| c.text.to_string())
        .collect();
    assert_eq!(
        names,
        ["all", "default", "kube-system", "monitoring", "web"]
    );
}

#[test]
fn contexts_say_which_are_connected() {
    let env = Env::new();
    let contexts = candidates(Slot::Context, &env);
    assert_eq!(contexts.len(), 4);
    assert_eq!(&*contexts[0].text, "dev");
    assert_eq!(&*contexts[0].detail, "connected");
    assert_eq!(&*contexts[3].detail, "");
}

#[test]
fn nothing_to_complete_offers_nothing() {
    assert!(candidates(Slot::Nothing, &Env::new()).is_empty());
}

#[test]
fn accepting_replaces_the_typed_word_and_leaves_room_for_the_next() {
    let line = "pods kube-";
    assert_eq!(
        accept(line, &site(line), "kube-system"),
        "pods kube-system "
    );
    let line = "dep";
    assert_eq!(accept(line, &site(line), "deploy"), "deploy ");
    let line = "pods web @pr";
    assert_eq!(accept(line, &site(line), "prod-eu"), "pods web @prod-eu ");
    let line = "pods ";
    assert_eq!(accept(line, &site(line), "web"), "pods web ");
    let line = ":dep";
    assert_eq!(accept(line, &site(line), "deploy"), ":deploy ");
}
