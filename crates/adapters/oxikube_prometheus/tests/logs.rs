//! Secrets never reach log output, at any level, on success or failure.

mod common;

use std::io;
use std::sync::Arc;

use common::endpoint;
use oxikube_ports::secrets::SecretString;
use oxikube_ports::{PromAuth, PromSecret, PromqlEndpointPort};
use oxikube_prometheus::PromqlClient;
use parking_lot::Mutex;
use tracing_subscriber::fmt::MakeWriter;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Capture {
        self.clone()
    }
}

const TOKEN: &str = "tok-SuperSecret-9f8e7d";
const PASSWORD: &str = "hunter2-PassWord-1a2b";

#[tokio::test(flavor = "current_thread")]
async fn tokens_and_passwords_never_appear_in_logs_or_errors() {
    let capture = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(capture.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let server = MockServer::start().await;
    // The server even echoes the credential back in an error: it must still be scrubbed.
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(500)
                .set_body_string(format!("failed for Authorization: Bearer {TOKEN}")),
        )
        .mount(&server)
        .await;

    let client = PromqlClient::new();
    let mut errors = Vec::new();
    let bearer = endpoint(&server).with_auth(PromAuth::Bearer(PromSecret::Memory(
        SecretString::from(TOKEN.to_owned()),
    )));
    errors.push(client.instant_query(&bearer, "up", None).await.unwrap_err());
    let basic = endpoint(&server).with_auth(PromAuth::Basic {
        username: "alice".into(),
        password: PromSecret::Memory(SecretString::from(PASSWORD.to_owned())),
    });
    errors.push(client.instant_query(&basic, "up", None).await.unwrap_err());
    // A transport failure with credentials set.
    let dead = oxikube_ports::PromEndpoint::new("http://127.0.0.1:1").with_auth(PromAuth::Bearer(
        PromSecret::Memory(SecretString::from(TOKEN.to_owned())),
    ));
    errors.push(client.instant_query(&dead, "up", None).await.unwrap_err());

    let logs = String::from_utf8(capture.0.lock().clone()).unwrap();
    assert!(
        logs.contains("promql request"),
        "the capture works: {logs:?}"
    );

    let basic_b64 = "YWxpY2U6aHVudGVyMi1QYXNzV29yZC0xYTJi"; // base64 of the basic-auth pair: user, colon, password
    let rendered: Vec<String> = errors
        .iter()
        .flat_map(|e| [e.to_string(), format!("{e:?}")])
        .collect();
    for text in std::iter::once(&logs).chain(rendered.iter()) {
        for secret in [TOKEN, PASSWORD, basic_b64] {
            assert!(!text.contains(secret), "`{secret}` leaked into: {text}");
        }
    }
    assert!(
        !format!("{bearer:?}{basic:?}").contains(TOKEN),
        "endpoint Debug is safe too"
    );
}
