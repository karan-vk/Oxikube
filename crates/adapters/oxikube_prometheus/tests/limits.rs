//! Size limits and cancel-on-drop.

mod common;

use std::time::Duration;

use common::endpoint;
use oxikube_ports::{PromEndpoint, PromqlEndpointPort, PromqlError};
use oxikube_prometheus::PromqlClient;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn big_matrix(bytes: usize) -> String {
    let point = "[1700000000,\"1\"],";
    let n = bytes / point.len();
    format!(
        r#"{{"status":"success","data":{{"resultType":"matrix","result":[{{"metric":{{}},"values":[{}[1,"1"]]}}]}}}}"#,
        point.repeat(n)
    )
}

#[tokio::test]
async fn a_response_over_the_limit_is_rejected_not_buffered() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(big_matrix(200_000).into_bytes(), "application/json"),
        )
        .mount(&server)
        .await;
    let client = PromqlClient::new();

    let small = endpoint(&server).with_max_response_bytes(50_000);
    let err = client.instant_query(&small, "up", None).await.unwrap_err();
    assert!(
        matches!(&err, PromqlError::BadResponse(m) if m.contains("limit")),
        "{err:?}"
    );

    let roomy = endpoint(&server).with_max_response_bytes(1_000_000);
    let series = client.instant_query(&roomy, "up", None).await.unwrap();
    assert!(series[0].points.len() > 1000);
}

#[tokio::test]
async fn a_chunked_response_without_a_length_is_still_capped() {
    // No Content-Length: the cap is enforced while reading.
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
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n")
                    .await;
                let chunk = vec![b' '; 16 * 1024];
                for _ in 0..64 {
                    if socket.write_all(&chunk).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    let ep = PromEndpoint::new(format!("http://127.0.0.1:{port}")).with_max_response_bytes(100_000);
    let err = PromqlClient::new()
        .instant_query(&ep, "up", None)
        .await
        .unwrap_err();
    assert!(matches!(err, PromqlError::BadResponse(_)), "{err:?}");
}

#[tokio::test]
async fn dropping_the_call_closes_the_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await; // the request
        let _ = seen_tx.send(());
        // Never answer. The client dropping its future closes the socket, which reads as EOF.
        let n = socket.read(&mut buf).await.unwrap_or(0);
        let _ = closed_tx.send(n == 0);
    });

    let ep =
        PromEndpoint::new(format!("http://127.0.0.1:{port}")).with_timeout(Duration::from_secs(60));
    let client = PromqlClient::new();
    {
        let call = client.instant_query(&ep, "up", None);
        tokio::pin!(call);
        // Poll the call until the server has the request, then abandon it mid-flight.
        tokio::select! {
            result = &mut call => panic!("the server never answers: {result:?}"),
            seen = seen_rx => seen.unwrap(),
        }
    }

    let closed = tokio::time::timeout(Duration::from_secs(5), closed_rx)
        .await
        .expect("the server saw the connection close")
        .unwrap();
    assert!(closed);
}
