//! Classification of websocket-upgrade failures (pod exec, attach, port-forward).
//!
//! kube reports a refused upgrade as only the HTTP status of the answer, so the status picks
//! the kind. The message never echoes the request URL or headers: it carries the status code
//! and a fixed hint, nothing from the wire.

use kube::client::UpgradeConnectionError;
use oxikube_domain::OxiError;
use oxikube_domain::redact::redact;

use super::CredentialRefresh;
use super::text::one_line;

/// Classifies an [`UpgradeConnectionError`].
///
/// | upgrade failure | [`OxiError`] |
/// |---|---|
/// | answered 401 | `Auth`, retryable when the credential can be refreshed |
/// | answered 403 | `Forbidden` (the identity lacks `create` on the streaming subresource) |
/// | answered 404 | `NotFound` |
/// | any other status, header mismatches, a broken connection | `Network` |
pub(super) fn classify_upgrade(
    err: &UpgradeConnectionError,
    refresh: CredentialRefresh,
) -> OxiError {
    let UpgradeConnectionError::ProtocolSwitch(status) = err else {
        return generic(err);
    };
    match status.as_u16() {
        401 => {
            let retryable = refresh.allows_retry();
            OxiError::auth(
                if retryable {
                    "the cluster rejected the credentials for the streaming connection (they may have expired)"
                } else {
                    "the cluster rejected the credentials for the streaming connection; sign in again or update the kubeconfig"
                },
                retryable,
            )
        }
        403 => OxiError::forbidden(
            "not allowed to open a streaming connection (exec, attach or port-forward): \
             the cluster answered 403 Forbidden; check RBAC for `create` on `pods/exec`, \
             `pods/attach` or `pods/portforward`",
        ),
        404 => OxiError::not_found(
            "the pod for the streaming connection (exec, attach or port-forward) was not found",
        ),
        _ => generic(err),
    }
}

fn generic(err: &UpgradeConnectionError) -> OxiError {
    OxiError::network(format!(
        "websocket upgrade failed: {}",
        one_line(&redact(&err.to_string()))
    ))
}

#[cfg(test)]
mod tests {
    use http::StatusCode;
    use oxikube_domain::ErrorKind;

    use super::*;

    fn switch(code: u16) -> UpgradeConnectionError {
        UpgradeConnectionError::ProtocolSwitch(StatusCode::from_u16(code).unwrap())
    }

    #[test]
    fn forbidden_upgrade_is_forbidden_and_not_retryable() {
        let e = classify_upgrade(&switch(403), CredentialRefresh::Unknown);
        assert_eq!(e.kind(), ErrorKind::Forbidden, "{e:?}");
        assert!(!e.is_retryable());
        assert!(e.message().contains("pods/exec"), "{e}");
    }

    #[test]
    fn unauthorized_upgrade_is_auth_retryable_per_the_credential() {
        for (refresh, retryable) in [
            (CredentialRefresh::Refreshable, true),
            (CredentialRefresh::Unknown, true),
            (CredentialRefresh::Static, false),
        ] {
            let e = classify_upgrade(&switch(401), refresh);
            assert_eq!(e.kind(), ErrorKind::Auth, "{e:?}");
            assert_eq!(e.is_retryable(), retryable, "{refresh:?}");
        }
    }

    #[test]
    fn missing_pod_upgrade_is_not_found() {
        let e = classify_upgrade(&switch(404), CredentialRefresh::Unknown);
        assert_eq!(e.kind(), ErrorKind::NotFound, "{e:?}");
        assert!(!e.is_retryable());
    }

    #[test]
    fn other_statuses_and_variants_stay_network() {
        use UpgradeConnectionError as U;
        let others = [
            switch(200),
            switch(500),
            switch(503),
            U::MissingUpgradeWebSocketHeader,
            U::MissingConnectionUpgradeHeader,
            U::SecWebSocketAcceptKeyMismatch,
            U::SecWebSocketProtocolMismatch,
        ];
        for err in others {
            let e = classify_upgrade(&err, CredentialRefresh::Unknown);
            assert_eq!(e.kind(), ErrorKind::Network, "{err}: {e:?}");
            assert!(e.is_retryable(), "{err}");
        }
    }

    #[test]
    fn messages_carry_no_url_or_header_text() {
        for code in [401, 403, 404, 500] {
            for refresh in [CredentialRefresh::Static, CredentialRefresh::Unknown] {
                let text = classify_upgrade(&switch(code), refresh)
                    .to_string()
                    .to_lowercase();
                for leak in [
                    "http://",
                    "https://",
                    "authorization",
                    "bearer",
                    "sec-websocket-key",
                ] {
                    assert!(!text.contains(leak), "{code}: {text}");
                }
            }
        }
    }
}
