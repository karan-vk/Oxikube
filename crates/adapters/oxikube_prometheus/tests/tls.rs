//! TLS: an untrusted certificate is a `TlsError`; the insecure toggle and a custom CA both
//! make the same server work.

mod common;

use std::sync::Arc;
use std::time::Duration;

use oxikube_ports::{PromEndpoint, PromTls, PromqlEndpointPort, PromqlError};
use oxikube_prometheus::PromqlClient;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

const BODY: &str = r#"{"status":"success","data":{"resultType":"vector","result":[{"metric":{"job":"tls"},"value":[1,"1"]}]}}"#;

/// Starts an HTTPS server for `localhost` with a fresh self-signed certificate. Returns its port
/// and the certificate as PEM.
async fn https_server() -> (u16, String) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
        .expect("generate a self-signed localhost certificate");
    let pem = certified.cert.pem();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring supports the default protocol versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![certified.cert.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
                certified.signing_key.serialize_der(),
            )),
        )
        .expect("the generated certificate and key form a valid server identity");
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let port = listener
        .local_addr()
        .expect("a bound listener has an address")
        .port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(mut tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let mut buf = [0u8; 4096];
                let _ = tls.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{BODY}",
                    BODY.len()
                );
                let _ = tls.write_all(response.as_bytes()).await;
                let _ = tls.shutdown().await;
            });
        }
    });
    (port, pem)
}

fn endpoint(port: u16, tls: PromTls) -> PromEndpoint {
    PromEndpoint::new(format!("https://localhost:{port}"))
        .with_tls(tls)
        .with_timeout(Duration::from_secs(10))
}

#[tokio::test]
async fn an_untrusted_certificate_is_a_tls_error() {
    let (port, _) = https_server().await;
    let err = PromqlClient::new()
        .instant_query(&endpoint(port, PromTls::default()), "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::TlsError(_)), "{err:?}");
    assert!(
        !err.is_absent(),
        "a certificate problem is not 'no Prometheus'"
    );
}

#[tokio::test]
async fn insecure_skip_verify_accepts_it() {
    let (port, _) = https_server().await;
    let tls = PromTls {
        insecure_skip_verify: true,
        ca_pem: None,
    };
    let series = PromqlClient::new()
        .instant_query(&endpoint(port, tls), "up", None)
        .await
        .unwrap();
    assert_eq!(series[0].labels["job"], "tls");
}

#[tokio::test]
async fn a_custom_ca_makes_it_trusted() {
    let (port, pem) = https_server().await;
    let tls = PromTls {
        insecure_skip_verify: false,
        ca_pem: Some(pem.into_bytes()),
    };
    let series = PromqlClient::new()
        .instant_query(&endpoint(port, tls), "up", None)
        .await
        .unwrap();
    assert_eq!(series.len(), 1);
}

#[tokio::test]
async fn a_wrong_ca_is_still_a_tls_error_and_garbage_is_an_invalid_endpoint() {
    let (port, _) = https_server().await;
    let (_, other_pem) = https_server().await;
    let wrong = PromTls {
        insecure_skip_verify: false,
        ca_pem: Some(other_pem.into_bytes()),
    };
    let err = PromqlClient::new()
        .instant_query(&endpoint(port, wrong), "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::TlsError(_)), "{err:?}");

    let garbage = PromTls {
        insecure_skip_verify: false,
        ca_pem: Some(b"not a certificate".to_vec()),
    };
    let err = PromqlClient::new()
        .instant_query(&endpoint(port, garbage), "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::InvalidEndpoint(_)), "{err:?}");
}

#[tokio::test]
async fn https_to_a_plain_http_server_is_a_tls_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let _ = socket
                    .write_all(b"HTTP/1.1 400 Bad Request\r\ncontent-length: 0\r\n\r\n")
                    .await;
            });
        }
    });
    let ep = PromEndpoint::new(format!("https://127.0.0.1:{port}"))
        .with_timeout(Duration::from_secs(10));
    let err = PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::TlsError(_)), "{err:?}");
}
