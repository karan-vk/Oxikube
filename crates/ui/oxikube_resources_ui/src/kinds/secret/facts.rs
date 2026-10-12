//! What a Secret's type puts on display (E12-S11), drawn from the [`SecretSummary`] the detail's
//! full read keeps: public facts only. No password, no `auth` string, no private key and no token
//! is in the summary, so none can be drawn here.

use gpui::{
    AnyElement, InteractiveElement as _, IntoElement as _, ParentElement as _, Styled as _, div, px,
};
use jiff::Timestamp;
use oxikube_app::{SecretSummary, TlsInfo, TypeSummary};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{Tokens, u};

use crate::table::ToneColors;

/// How many days before it expires a certificate is shown as a warning.
const EXPIRY_WARNING_DAYS: i64 = 30;

/// The facts of the type as rows (`label`, `value`, tone), `None` for a type with none.
pub(super) fn fact_rows(
    tokens: &Tokens,
    tones: &ToneColors,
    summary: &SecretSummary,
    now: Timestamp,
) -> Option<AnyElement> {
    let lines: Vec<AnyElement> = match &summary.details {
        TypeSummary::Plain | TypeSummary::SshAuth => return None,
        TypeSummary::Registry(logins) => logins
            .iter()
            .enumerate()
            .map(|(index, login)| {
                let who = match (&login.username, login.has_credentials) {
                    (Some(user), _) => user.clone(),
                    (None, true) => "credentials set".to_owned(),
                    (None, false) => "no credentials".to_owned(),
                };
                fact(
                    tokens,
                    "registry",
                    index,
                    "registry",
                    &format!("{} · {who}", login.server),
                    None,
                )
            })
            .collect(),
        TypeSummary::RegistryUnreadable => vec![fact(
            tokens,
            "registry",
            0,
            "unreadable",
            "the config is not valid JSON",
            Some(tones.warn),
        )],
        TypeSummary::Tls(info) => tls_lines(tokens, tones, info, now),
        TypeSummary::TlsUnreadable(reason) => vec![fact(
            tokens,
            "certificate",
            0,
            "unreadable",
            reason,
            Some(tones.warn),
        )],
        TypeSummary::BasicAuth { username } => vec![fact(
            tokens,
            "user",
            0,
            "username",
            username.as_deref().unwrap_or("not text"),
            None,
        )],
        TypeSummary::ServiceAccountToken { service_account } => vec![fact(
            tokens,
            "account",
            0,
            "service account",
            service_account.as_deref().unwrap_or("unknown"),
            None,
        )],
    };
    Some(v_flex().gap(u(px(2.))).children(lines).into_any_element())
}

fn tls_lines(
    tokens: &Tokens,
    tones: &ToneColors,
    info: &TlsInfo,
    now: Timestamp,
) -> Vec<AnyElement> {
    let days = info.days_left(now);
    let (relative, tone) = if info.is_expired(now) {
        (format!("expired {} days ago", -days), Some(tones.error))
    } else if days <= EXPIRY_WARNING_DAYS {
        (format!("in {days} days"), Some(tones.warn))
    } else {
        (format!("in {days} days"), None)
    };
    let mut lines = vec![
        fact(tokens, "tls", 0, "subject", &info.subject, None),
        fact(
            tokens,
            "tls",
            1,
            "valid until",
            &format!("{} ({relative})", date(info.not_after)),
            tone,
        ),
        fact(tokens, "tls", 2, "issuer", &info.issuer, None),
    ];
    if !info.dns_names.is_empty() {
        lines.push(fact(
            tokens,
            "tls",
            3,
            "names",
            &info.dns_names.join(", "),
            None,
        ));
    }
    if info.certificates > 1 {
        lines.push(fact(
            tokens,
            "tls",
            4,
            "chain",
            &format!("{} certificates", info.certificates),
            None,
        ));
    }
    lines
}

/// `2026-12-31`.
fn date(at: Timestamp) -> String {
    at.to_string().chars().take(10).collect()
}

/// One `label  value  note` line.
fn fact(
    tokens: &Tokens,
    group: &'static str,
    index: usize,
    label: &str,
    value: &str,
    tone: Option<gpui::Hsla>,
) -> AnyElement {
    let colors = tokens.colors;
    h_flex()
        .debug_selector(move || format!("secret-fact-{group}-{index}"))
        .gap(u(tokens.spacing.md))
        .px(u(tokens.spacing.lg))
        .text_size(u(tokens.font.small))
        .child(
            div()
                .flex_none()
                .w(u(px(88.)))
                .text_color(colors.text_muted)
                .child(label.to_owned()),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(tone.unwrap_or(colors.text))
                .child(value.to_owned()),
        )
        .into_any_element()
}
