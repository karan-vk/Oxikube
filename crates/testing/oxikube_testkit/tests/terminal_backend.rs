//! Worked example: a service that only knows `Arc<dyn ExecPort>` and `dyn TerminalBackend`
//! is driven end to end by `FakeExecPort` and `FakeTerminalBackend`. This is the shape
//! `ExecService` (E09-S08) and the terminal tab (E09-S07) are tested in.

use std::sync::Arc;

use futures::StreamExt;
use futures::executor::block_on;
use oxikube_domain::ids::{ClusterId, ContextName, Gvk, ResourceRef};
use oxikube_ports::{
    BackendEvent, ExecPort, ExecTarget, ExitStatus, TerminalBackend, TerminalSize,
};
use oxikube_testkit::{ExecPortCall, FakeExecPort, FakeTerminalBackend};

/// What a terminal tab does with a backend: type a line, collect what comes back until the
/// session ends.
struct Session {
    backend: Box<dyn TerminalBackend>,
}

impl Session {
    async fn open(port: &Arc<dyn ExecPort>, target: &ExecTarget) -> Self {
        let backend = port.exec(target).await.expect("open");
        backend
            .resize(TerminalSize::new(100, 30))
            .await
            .expect("resize");
        Self { backend }
    }

    async fn run(&self, line: &str) -> (String, Option<ExitStatus>) {
        let mut events = self.backend.output_stream();
        self.backend.write(line.as_bytes()).await.expect("write");
        let mut out = Vec::new();
        let mut exit = None;
        while let Some(event) = events.next().await {
            match event {
                BackendEvent::Output(bytes) => {
                    out.extend_from_slice(&bytes);
                    if out.ends_with(b"\n") {
                        // The test ends the session when the line came back.
                        self.backend.kill().await.expect("kill");
                    }
                }
                BackendEvent::Exited(status) => exit = Some(status),
                BackendEvent::Error(error) => panic!("transport error: {error}"),
            }
        }
        (String::from_utf8(out).expect("utf8"), exit)
    }
}

#[test]
fn a_service_drives_the_fake_end_to_end() {
    block_on(async {
        let fake = Arc::new(FakeExecPort::new());
        let remote = FakeTerminalBackend::echo();
        fake.script().exec.push_ok(remote.clone());
        let port: Arc<dyn ExecPort> = fake.clone();

        let pod = ResourceRef::new(
            ClusterId::new("kubeconfig", &ContextName::new("kind")),
            Gvk::new("", "v1", "Pod"),
            Some("demo".into()),
            "web-0",
        );
        let target = ExecTarget::interactive(pod, vec!["sh".into()]).container("app");
        let session = Session::open(&port, &target).await;
        let (output, exit) = session.run("echo hi\n").await;

        assert_eq!(output, "echo hi\n");
        assert_eq!(exit.and_then(|s| s.signal).as_deref(), Some("KILL"));
        assert_eq!(remote.resizes(), vec![TerminalSize::new(100, 30)]);
        assert_eq!(remote.kill_count(), 1);
        assert_eq!(fake.recorded_calls(), vec![ExecPortCall::Exec(target)]);
    });
}
