//! What the guard says about `serviceaccount::GenerateKubeconfig`: the confirmation text and the
//! audit detail. Neither can hold a token: the command carries none.

use oxikube_domain::command::Command;

use super::token::DEFAULT_EXPIRATION_SECONDS;

/// The confirmation text: the account, the namespace, the lifetime and the cluster.
pub(crate) fn summary(command: &Command, context: &str) -> Option<String> {
    let Command::ServiceAccountGenerateKubeconfig {
        target,
        expiration_seconds,
        allow_legacy_token,
        ..
    } = command
    else {
        return None;
    };
    let namespace = target.namespace().unwrap_or_default();
    let lifetime = format_lifetime(expiration_seconds.unwrap_or(DEFAULT_EXPIRATION_SECONDS));
    let fallback = if *allow_legacy_token {
        " On a cluster without TokenRequest a long-lived token Secret may be read or created."
    } else {
        ""
    };
    Some(format!(
        "Create a token for ServiceAccount {namespace}/{} on {context}, valid for {lifetime}, \
         and build a kubeconfig from it.{fallback}",
        target.name
    ))
}

/// The audit detail: the lifetime asked for, the audiences' count and whether the legacy
/// fallback was allowed. The account is the record's target; the token is never known here.
pub(crate) fn audit_detail(command: &Command) -> Option<String> {
    let Command::ServiceAccountGenerateKubeconfig {
        expiration_seconds,
        audiences,
        allow_legacy_token,
        ..
    } = command
    else {
        return None;
    };
    Some(format!(
        "expiration={}s audiences={} legacy_allowed={allow_legacy_token}",
        expiration_seconds.unwrap_or(DEFAULT_EXPIRATION_SECONDS),
        audiences.len()
    ))
}

/// `3600` as "1 hour", `600` as "10 minutes", `90` as "90 seconds".
pub fn format_lifetime(seconds: u32) -> String {
    const DAY: u32 = 86_400;
    const HOUR: u32 = 3_600;
    const MINUTE: u32 = 60;
    let (count, unit) = if seconds >= DAY && seconds.is_multiple_of(DAY) {
        (seconds / DAY, "day")
    } else if seconds >= HOUR && seconds.is_multiple_of(HOUR) {
        (seconds / HOUR, "hour")
    } else if seconds >= MINUTE && seconds.is_multiple_of(MINUTE) {
        (seconds / MINUTE, "minute")
    } else {
        (seconds, "second")
    };
    format!("{count} {unit}{}", if count == 1 { "" } else { "s" })
}
