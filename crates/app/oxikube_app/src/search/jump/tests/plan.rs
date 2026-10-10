//! From a line to navigation commands, against the test environment.

use oxikube_domain::AliasTarget;
use oxikube_domain::command::Command;
use oxikube_domain::ids::{Gvk, Gvr};

use super::super::{CATALOG_VIEW_ID, JumpPlan, ParseErrorKind, plan};
use super::{Env, cluster};

fn planned(line: &str) -> JumpPlan {
    plan(line, &Env::new()).unwrap_or_else(|e| panic!("`{line}`: {e}"))
}

fn failed(line: &str) -> super::super::ParseError {
    plan(line, &Env::new()).expect_err(line)
}

fn open(group: &str, version: &str, kind: &str) -> Command {
    Command::ResourceOpenList {
        cluster: cluster("dev"),
        gvk: Gvk::new(group, version, kind),
    }
}

fn set_filter(group: &str, version: &str, kind: &str, text: &str) -> Command {
    Command::TableSetFilter {
        cluster: cluster("dev"),
        gvk: Gvk::new(group, version, kind),
        text: text.to_owned(),
    }
}

#[test]
fn a_resource_opens_its_list_in_the_shown_cluster() {
    assert_eq!(planned("pods").commands, [open("", "v1", "Pod")]);
    assert_eq!(planned("po").commands, [open("", "v1", "Pod")]);
    assert_eq!(
        planned("Deployments").commands,
        [open("apps", "v1", "Deployment")]
    );
}

#[test]
fn a_namespace_is_selected_before_the_list_opens() {
    assert_eq!(
        planned("deploy kube-system").commands,
        [
            Command::NamespaceSelect {
                cluster: cluster("dev"),
                namespaces: vec!["kube-system".to_owned()],
            },
            open("apps", "v1", "Deployment"),
        ]
    );
}

#[test]
fn all_is_every_namespace() {
    assert_eq!(
        planned("pods all").commands[0],
        Command::NamespaceSelect {
            cluster: cluster("dev"),
            namespaces: Vec::new(),
        }
    );
}

#[test]
fn a_filter_is_applied_to_the_table_that_opens() {
    assert_eq!(
        planned("pod /re").commands,
        [open("", "v1", "Pod"), set_filter("", "v1", "Pod", "re")]
    );
    assert_eq!(
        planned("pod /!re").commands[1],
        set_filter("", "v1", "Pod", "!re")
    );
    assert_eq!(
        planned("pod /").commands[1],
        set_filter("", "v1", "Pod", ""),
        "a bare slash clears the filter"
    );
}

#[test]
fn a_selector_becomes_the_filter_bars_label_flag() {
    assert_eq!(
        planned("pod app=nginx").commands,
        [
            open("", "v1", "Pod"),
            set_filter("", "v1", "Pod", "-l app=nginx"),
        ]
    );
    assert_eq!(
        planned("pod app=x,env=y").commands[1],
        set_filter("", "v1", "Pod", "-l app=x,env=y")
    );
}

#[test]
fn a_filter_and_a_selector_apply_together() {
    assert_eq!(
        planned("pod /api app=x").commands[1],
        set_filter("", "v1", "Pod", "api -l app=x")
    );
    assert_eq!(
        planned("pod /!-f fz app=x").commands[1],
        set_filter("", "v1", "Pod", "!-f fz -l app=x")
    );
    assert_eq!(
        planned("pod /-l tier=web app=x").commands[1],
        set_filter("", "v1", "Pod", "-l tier=web,app=x"),
        "two selectors are one list"
    );
}

#[test]
fn every_token_together() {
    let plan = planned("deploy kube-system /api app=x @prod-eu");
    let prod = cluster("prod-eu");
    assert_eq!(
        plan.commands,
        [
            Command::ClusterSelect {
                cluster: prod.clone()
            },
            Command::NamespaceSelect {
                cluster: prod.clone(),
                namespaces: vec!["kube-system".to_owned()],
            },
            Command::ResourceOpenList {
                cluster: prod.clone(),
                gvk: Gvk::new("apps", "v1", "Deployment"),
            },
            Command::TableSetFilter {
                cluster: prod,
                gvk: Gvk::new("apps", "v1", "Deployment"),
                text: "api -l app=x".to_owned(),
            },
        ]
    );
    assert!(plan.after_connect.is_none());
}

#[test]
fn a_context_that_is_not_connected_connects_first() {
    let plan = planned("pods @staging");
    let staging = cluster("staging");
    assert_eq!(
        plan.commands,
        [Command::ClusterConnect {
            cluster: staging.clone()
        }]
    );
    let after = plan
        .after_connect
        .expect("the rest waits for the connection");
    assert_eq!(after.cluster, staging);
    assert_eq!(
        after.commands,
        [
            Command::ClusterSelect {
                cluster: staging.clone()
            },
            Command::ResourceOpenList {
                cluster: staging,
                gvk: Gvk::new("", "v1", "Pod"),
            },
        ]
    );
}

#[test]
fn a_context_matches_by_exact_name_then_case_then_a_unique_prefix() {
    for word in ["prod-eu", "PROD-EU", "prod-e"] {
        let plan = planned(&format!("pods @{word}"));
        assert_eq!(
            plan.commands[0],
            Command::ClusterSelect {
                cluster: cluster("prod-eu")
            },
            "@{word}"
        );
    }
}

#[test]
fn the_alias_table_of_the_target_context_decides() {
    // `certs` is a CRD of `dev` only.
    assert_eq!(
        planned("certs").commands,
        [open("cert-manager.io", "v1", "Certificate")]
    );
    let e = failed("certs @prod-eu");
    assert_eq!(e.kind, ParseErrorKind::UnknownAlias);
    assert_eq!(&"certs @prod-eu"[e.span.range()], "certs");
}

#[test]
fn crd_aliases_open_the_crds_list() {
    for word in [
        "certs",
        "cert",
        "certificate",
        "certificates",
        "certificates.cert-manager.io",
    ] {
        assert_eq!(
            planned(word).commands,
            [open("cert-manager.io", "v1", "Certificate")],
            "{word}"
        );
    }
    assert_eq!(
        planned("crd").commands,
        [open(
            "apiextensions.k8s.io",
            "v1",
            "CustomResourceDefinition"
        )]
    );
}

#[test]
fn ctx_without_a_name_shows_the_catalog() {
    assert_eq!(
        planned("ctx").commands,
        [Command::ViewOpen {
            view: CATALOG_VIEW_ID.to_owned()
        }]
    );
}

#[test]
fn ctx_with_a_name_shows_the_tab_or_connects() {
    assert_eq!(
        planned("ctx prod-eu").commands,
        [Command::ClusterSelect {
            cluster: cluster("prod-eu")
        }]
    );
    let plan = planned("ctx staging");
    assert_eq!(
        plan.commands,
        [Command::ClusterConnect {
            cluster: cluster("staging")
        }]
    );
    assert_eq!(
        plan.after_connect.unwrap().commands,
        [Command::ClusterSelect {
            cluster: cluster("staging")
        }]
    );
}

#[test]
fn ns_opens_the_namespaces_list_or_selects_one() {
    assert_eq!(planned("ns").commands, [open("", "v1", "Namespace")]);
    assert_eq!(
        planned("ns monitoring").commands,
        [Command::NamespaceSelect {
            cluster: cluster("dev"),
            namespaces: vec!["monitoring".to_owned()],
        }]
    );
}

#[test]
fn q_is_the_quit_command_which_asks_before_losing_work() {
    assert_eq!(planned("q").commands, [Command::AppQuit]);
}

#[test]
fn history_words_are_history_commands() {
    assert_eq!(planned("-").commands, [Command::JumpLast]);
    assert_eq!(planned("[").commands, [Command::JumpBack]);
    assert_eq!(planned("]").commands, [Command::JumpForward]);
}

#[test]
fn only_navigation_is_recorded_in_canonical_form() {
    assert_eq!(
        planned("deploy @prod-eu app=x /api kube-system")
            .record
            .as_deref(),
        Some("deploy kube-system /api app=x @prod-eu")
    );
    assert_eq!(
        planned("ctx prod-eu").record.as_deref(),
        Some("ctx prod-eu")
    );
    assert_eq!(planned("q").record, None);
    assert_eq!(planned("[").record, None);
}

#[test]
fn an_unknown_alias_suggests_close_names() {
    let e = failed("podz");
    assert_eq!(e.kind, ParseErrorKind::UnknownAlias);
    assert!(
        e.suggestions.iter().any(|s| &**s == "pods"),
        "{:?}",
        e.suggestions
    );
    assert!(e.message.contains("podz") && e.message.contains("pods"));
    let e = failed("deplo kube-system");
    assert_eq!(&"deplo kube-system"[e.span.range()], "deplo");
    assert!(e.suggestions.iter().any(|s| s.starts_with("deploy")));
}

#[test]
fn an_unknown_namespace_is_an_error_against_the_clusters_own_list() {
    let line = "pods kube-systm";
    let e = failed(line);
    assert_eq!(e.kind, ParseErrorKind::UnknownNamespace);
    assert_eq!(&line[e.span.range()], "kube-systm");
    assert_eq!(e.suggestions.first().map(|s| &**s), Some("kube-system"));
}

#[test]
fn a_namespace_list_that_is_not_the_clusters_own_accepts_any_valid_name() {
    use crate::session::namespaces::{NamespaceCatalog, NamespaceSource};
    let mut env = Env::new();
    env.namespaces.insert(
        cluster("dev"),
        NamespaceCatalog {
            names: vec!["typed-by-user".to_owned()],
            source: NamespaceSource::Forbidden,
        },
    );
    let plan = plan("pods anywhere", &env).unwrap();
    assert_eq!(
        plan.commands[0],
        Command::NamespaceSelect {
            cluster: cluster("dev"),
            namespaces: vec!["anywhere".to_owned()],
        }
    );
    // Not read yet: also accepted.
    env.namespaces.clear();
    assert!(super::super::plan("pods anywhere", &env).is_ok());
}

#[test]
fn a_namespace_named_all_wins_over_the_keyword() {
    use crate::session::namespaces::{NamespaceCatalog, NamespaceSource};
    let mut env = Env::new();
    env.namespaces.insert(
        cluster("dev"),
        NamespaceCatalog {
            names: vec!["all".to_owned()],
            source: NamespaceSource::Cluster,
        },
    );
    let plan = plan("pods all", &env).unwrap();
    assert_eq!(
        plan.commands[0],
        Command::NamespaceSelect {
            cluster: cluster("dev"),
            namespaces: vec!["all".to_owned()],
        }
    );
}

#[test]
fn an_unknown_context_suggests_close_names() {
    let line = "pods @prd-eu";
    let e = failed(line);
    assert_eq!(e.kind, ParseErrorKind::UnknownContext);
    assert_eq!(&line[e.span.range()], "@prd-eu");
    assert!(e.suggestions.iter().any(|s| &**s == "prod-eu"));
}

#[test]
fn an_ambiguous_context_prefix_lists_the_candidates() {
    let e = failed("ctx prod");
    assert_eq!(e.kind, ParseErrorKind::AmbiguousContext);
    assert_eq!(
        e.suggestions.iter().map(|s| &**s).collect::<Vec<_>>(),
        ["prod-eu", "prod-us"]
    );
}

#[test]
fn without_a_cluster_tab_there_is_nothing_to_jump_in_but_ctx_and_q_still_work() {
    let mut env = Env::new();
    env.active = None;
    let e = plan("pods", &env).unwrap_err();
    assert_eq!(e.kind, ParseErrorKind::NoCluster);
    assert_eq!(
        plan("ns", &env).unwrap_err().kind,
        ParseErrorKind::NoCluster
    );
    assert!(plan("ctx", &env).is_ok());
    assert!(plan("ctx prod-eu", &env).is_ok());
    assert!(plan("q", &env).is_ok());
    assert!(
        plan("pods @prod-eu", &env).is_ok(),
        "a context names the cluster"
    );
}

#[test]
fn a_type_nobody_knows_the_kind_of_is_not_served() {
    let env = Env::new();
    env.tables[&cluster("dev")].set_user_aliases([(
        "ghosts".to_owned(),
        AliasTarget::Gvr(Gvr::new("ghost.io", "v1", "ghosts")),
    )]);
    let e = plan("ghosts", &env).unwrap_err();
    assert_eq!(e.kind, ParseErrorKind::NotServed);
}

#[test]
fn a_user_alias_for_a_command_line_expands_and_keeps_what_follows_it() {
    let env = Env::new();
    env.tables[&cluster("dev")].set_user_aliases([
        (
            "fred".to_owned(),
            AliasTarget::command("pod", ["web".to_owned(), "app=blee".to_owned()]),
        ),
        ("ing2".to_owned(), AliasTarget::command("ing", [])),
    ]);
    let plan1 = plan("fred", &env).unwrap();
    assert_eq!(
        plan1.commands,
        [
            Command::NamespaceSelect {
                cluster: cluster("dev"),
                namespaces: vec!["web".to_owned()],
            },
            open("", "v1", "Pod"),
            set_filter("", "v1", "Pod", "-l app=blee"),
        ]
    );
    // What the user types after the alias is added: a filter joins the stored selector.
    let plan2 = plan("fred /api", &env).unwrap();
    assert_eq!(
        plan2.commands[2],
        set_filter("", "v1", "Pod", "api -l app=blee")
    );
    // The alias word is what the history keeps.
    assert_eq!(plan1.record.as_deref(), Some("fred"));
    // A second namespace conflicts with the stored one, and the error is on the word typed.
    let e = plan("fred kube-system", &env).unwrap_err();
    assert_eq!(e.kind, ParseErrorKind::DuplicateNamespace);
    assert_eq!(&"fred kube-system"[e.span.range()], "fred");
    assert_eq!(
        plan("ing2", &env).unwrap().commands,
        [open("networking.k8s.io", "v1", "Ingress")]
    );
}

#[test]
fn an_alias_that_expands_into_itself_is_a_loop_error_not_a_hang() {
    let env = Env::new();
    env.tables[&cluster("dev")].set_user_aliases([
        ("a".to_owned(), AliasTarget::command("b", [])),
        ("b".to_owned(), AliasTarget::command("a", [])),
    ]);
    let e = plan("a", &env).unwrap_err();
    assert_eq!(e.kind, ParseErrorKind::AliasLoop);
}
