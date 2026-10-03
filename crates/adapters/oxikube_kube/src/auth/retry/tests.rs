use std::cell::Cell;

use kube::core::Status;

use super::*;

fn api_error(code: u16) -> kube::Error {
    kube::Error::Api(Box::new(Status::failure("rejected", "").with_code(code)))
}

#[tokio::test]
async fn success_does_not_invalidate() {
    let (calls, invalidations) = (Cell::new(0), Cell::new(0));
    let out = retry_once(
        || async { invalidations.set(invalidations.get() + 1) },
        || async {
            calls.set(calls.get() + 1);
            Ok::<_, OxiError>(7)
        },
    )
    .await;
    assert_eq!(out.unwrap(), 7);
    assert_eq!((calls.get(), invalidations.get()), (1, 0));
}

#[tokio::test]
async fn first_401_then_success_rebuilds_once() {
    let (calls, invalidations) = (Cell::new(0), Cell::new(0));
    let out = retry_once_kube(
        CredentialRefresh::Refreshable,
        || async { invalidations.set(invalidations.get() + 1) },
        || async {
            calls.set(calls.get() + 1);
            if calls.get() == 1 {
                Err(api_error(401))
            } else {
                Ok("fresh")
            }
        },
    )
    .await;
    assert_eq!(out.unwrap(), "fresh");
    assert_eq!((calls.get(), invalidations.get()), (2, 1));
}

#[tokio::test]
async fn two_401s_stop_after_one_retry_and_are_final() {
    let (calls, invalidations) = (Cell::new(0), Cell::new(0));
    let err = retry_once_kube(
        CredentialRefresh::Refreshable,
        || async { invalidations.set(invalidations.get() + 1) },
        || async {
            calls.set(calls.get() + 1);
            Err::<(), _>(api_error(401))
        },
    )
    .await
    .unwrap_err();
    assert_eq!((calls.get(), invalidations.get()), (2, 1), "never loops");
    assert_eq!(err.kind(), ErrorKind::Auth);
    assert!(!err.is_retryable(), "a failed refresh is final");
}

#[tokio::test]
async fn static_credential_401_is_not_retried() {
    let (calls, invalidations) = (Cell::new(0), Cell::new(0));
    let err = retry_once_kube(
        CredentialRefresh::Static,
        || async { invalidations.set(invalidations.get() + 1) },
        || async {
            calls.set(calls.get() + 1);
            Err::<(), _>(api_error(401))
        },
    )
    .await
    .unwrap_err();
    assert_eq!((calls.get(), invalidations.get()), (1, 0));
    assert_eq!(err.kind(), ErrorKind::Auth);
}

#[tokio::test]
async fn non_auth_errors_are_not_retried() {
    for code in [403, 404, 503] {
        let (calls, invalidations) = (Cell::new(0), Cell::new(0));
        let err = retry_once_kube(
            CredentialRefresh::Refreshable,
            || async { invalidations.set(invalidations.get() + 1) },
            || async {
                calls.set(calls.get() + 1);
                Err::<(), _>(api_error(code))
            },
        )
        .await
        .unwrap_err();
        assert_eq!((calls.get(), invalidations.get()), (1, 0), "{code}");
        assert_ne!(err.kind(), ErrorKind::Auth);
    }
}

#[tokio::test]
async fn retry_surfaces_a_different_error_from_the_second_attempt() {
    let calls = Cell::new(0);
    let err = retry_once_kube(
        CredentialRefresh::Unknown,
        || async {},
        || async {
            calls.set(calls.get() + 1);
            Err::<(), _>(api_error(if calls.get() == 1 { 401 } else { 403 }))
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Forbidden);
}
