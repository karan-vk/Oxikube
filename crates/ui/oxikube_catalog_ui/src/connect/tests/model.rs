//! The view model and the text helpers: every state's content, redaction, truncation. No window.

use oxikube_domain::session::ClusterSessionState;
use oxikube_ports::ExecInteractivity;

use crate::connect::model::{ConnectInfo, ConnectViewModel, DegradedModel, TerminalAction};
use crate::connect::policy::ExecPolicy;
use crate::connect::text::{DisplayText, SUMMARY_MAX_CHARS, shorten};

/// A made-up bearer token, as a plugin might echo it in its stderr.
pub(super) const TOKEN: &str =
    "eyJhbGciOiJSUzI1NiIsImtpZCI6ImFiYyJ9.eyJzdWIiOiJ1c2VyIiwiZXhwIjoxfQ.c2lnbmF0dXJlc2lnbmF0dXJl";

fn info() -> ConnectInfo {
    ConnectInfo {
        title: "prod-eu".into(),
        context: "prod-eu-ctx".into(),
        server: Some("https://prod.example:6443".into()),
        exec: ExecInteractivity::Never,
        terminal: true,
        sources: true,
    }
}

fn auth(reason: &str) -> ClusterSessionState {
    ClusterSessionState::AuthRequired {
        reason: reason.into(),
    }
}

fn error(reason: &str) -> ClusterSessionState {
    ClusterSessionState::Error {
        reason: reason.into(),
    }
}

#[test]
fn ready_shows_the_clusters_own_content_and_nothing_of_ours() {
    let model = ConnectViewModel::of(&ClusterSessionState::Ready, &info());
    assert_eq!(model, ConnectViewModel::Content);
    assert!(!model.replaces_content());
    assert!(model.banner().is_none());
}

#[test]
fn connecting_names_the_server_and_the_context() {
    let model = ConnectViewModel::of(&ClusterSessionState::Connecting, &info());
    let ConnectViewModel::Connecting(connecting) = &model else {
        panic!("{model:?}");
    };
    assert_eq!(connecting.title, "prod-eu");
    assert_eq!(connecting.context, "prod-eu-ctx");
    assert_eq!(
        connecting.server.as_deref(),
        Some("https://prod.example:6443")
    );
    assert!(model.replaces_content());
}

#[test]
fn auth_required_carries_the_plugin_message_the_policy_and_the_terminal_action() {
    let model = ConnectViewModel::of(&auth("token expired"), &info());
    let ConnectViewModel::AuthRequired(auth) = &model else {
        panic!("{model:?}");
    };
    assert_eq!(auth.message.summary, "token expired");
    assert!(!auth.message.truncated);
    assert_eq!(auth.policy.label(), "Forbid");
    assert_eq!(auth.terminal, TerminalAction::Available);
    assert!(
        auth.instructions.contains("forbidden"),
        "{}",
        auth.instructions
    );
    assert!(model.replaces_content());

    let no_terminal = ConnectInfo {
        terminal: false,
        ..info()
    };
    let model = ConnectViewModel::of(&auth_state(), &no_terminal);
    let ConnectViewModel::AuthRequired(auth) = &model else {
        panic!("{model:?}");
    };
    assert_eq!(auth.terminal, TerminalAction::Unavailable);
    assert!(
        auth.instructions.contains("outside Oxikube"),
        "forbidden without a terminal says how to sign in elsewhere: {}",
        auth.instructions
    );
}

fn auth_state() -> ClusterSessionState {
    auth("the exec plugin needs you to sign in")
}

#[test]
fn the_exec_policy_is_named_for_every_setting() {
    let table = [
        (ExecInteractivity::Never, "Forbid", "never", false),
        (ExecInteractivity::IfAvailable, "Ask", "if_available", true),
        (ExecInteractivity::Always, "Allow", "always", true),
    ];
    for (setting, label, value, interactive) in table {
        let policy = ExecPolicy::of(setting);
        assert_eq!(policy.label(), label);
        assert_eq!(policy.setting_value(), value);
        assert_eq!(policy.allows_interaction(), interactive);
        assert!(!policy.explanation().is_empty());
        for terminal in [true, false] {
            let text = policy.instructions(terminal);
            assert_eq!(text.contains("forbidden"), !interactive, "{text}");
        }
    }
}

#[test]
fn the_policy_follows_the_cluster_into_the_auth_model() {
    for setting in [
        ExecInteractivity::Never,
        ExecInteractivity::IfAvailable,
        ExecInteractivity::Always,
    ] {
        let info = ConnectInfo {
            exec: setting,
            ..info()
        };
        let model = ConnectViewModel::of(&auth_state(), &info);
        let ConnectViewModel::AuthRequired(auth) = model else {
            panic!("auth");
        };
        assert_eq!(auth.policy.setting, setting);
    }
}

#[test]
fn degraded_is_a_banner_over_the_content_not_a_replacement() {
    let model = ConnectViewModel::of(&ClusterSessionState::Degraded, &info());
    assert!(!model.replaces_content());
    assert_eq!(
        model.banner(),
        Some(&DegradedModel {
            title: "prod-eu".into()
        })
    );
    assert!(DegradedModel::HEADLINE.contains("stale"));
}

#[test]
fn error_has_a_summary_details_and_the_way_to_the_sources() {
    let reason = "connection refused: dial tcp 10.0.0.1:6443\nretried 3 times\nlast: i/o timeout";
    let model = ConnectViewModel::of(&error(reason), &info());
    let ConnectViewModel::Error(error) = &model else {
        panic!("{model:?}");
    };
    assert_eq!(
        error.message.summary,
        "connection refused: dial tcp 10.0.0.1:6443"
    );
    assert!(error.message.truncated, "more lines than the summary shows");
    assert!(error.details.contains("cluster: prod-eu\n"));
    assert!(error.details.contains("context: prod-eu-ctx\n"));
    assert!(
        error
            .details
            .contains("server: https://prod.example:6443\n")
    );
    assert!(error.details.contains("exec_interactivity: never"));
    assert!(
        error.details.ends_with("last: i/o timeout"),
        "{}",
        error.details
    );
    assert!(error.sources);

    let hidden = ConnectInfo {
        sources: false,
        ..info()
    };
    let ConnectViewModel::Error(error) = ConnectViewModel::of(&super::model::error("x"), &hidden)
    else {
        panic!("error");
    };
    assert!(!error.sources, "no sources page, no link");
}

#[test]
fn disconnected_offers_connect() {
    let model = ConnectViewModel::of(&ClusterSessionState::Disconnected, &info());
    assert!(matches!(model, ConnectViewModel::Disconnected(_)));
    assert!(model.replaces_content());
}

#[test]
fn a_token_in_any_text_never_reaches_the_model() {
    let leaky = format!("exec plugin failed: Authorization: Bearer {TOKEN}\nstderr: token={TOKEN}");
    let with_url = ConnectInfo {
        server: Some("https://admin:hunter2secret@prod.example:6443".into()),
        ..info()
    };
    for state in [auth(&leaky), error(&leaky)] {
        let model = ConnectViewModel::of(&state, &with_url);
        let shown = format!("{model:?}");
        assert!(!shown.contains(TOKEN), "{shown}");
        assert!(!shown.contains("hunter2secret"), "{shown}");
    }
    // The server URL is scrubbed where the info is built from a session, too.
    let text = DisplayText::new(&leaky);
    assert!(!text.summary.contains(TOKEN));
    assert!(!text.full.contains(TOKEN));
}

#[test]
fn a_long_error_is_summarised_on_one_line_and_kept_whole_in_the_details() {
    let long = "x".repeat(2000);
    let text = DisplayText::new(&long);
    assert_eq!(text.summary.chars().count(), SUMMARY_MAX_CHARS);
    assert!(text.summary.ends_with('…'));
    assert_eq!(text.full, long);
    assert!(text.truncated);

    let short = DisplayText::new("  connection refused \n");
    assert_eq!(short.summary, "connection refused");
    assert!(!short.truncated);
}

#[test]
fn summaries_cut_on_character_boundaries() {
    let text = "ü".repeat(500);
    let cut = shorten(&text, 10);
    assert_eq!(cut.chars().count(), 10);
    assert!(cut.ends_with('…'));
    assert_eq!(shorten("short", 10), "short");
    let emoji = "🔒".repeat(300);
    assert!(DisplayText::new(&emoji).summary.chars().count() <= SUMMARY_MAX_CHARS);
}

#[test]
fn a_reason_that_is_only_blank_has_no_text() {
    assert!(DisplayText::new(" \n \t ").is_empty());
}
