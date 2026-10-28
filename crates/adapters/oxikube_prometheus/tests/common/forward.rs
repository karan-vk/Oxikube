//! `kubectl port-forward` to a pod, killed on drop (kind tests only).

use std::io::{BufRead, BufReader};
use std::process::{Child, ChildStdout, Command, Stdio};

pub struct PortForward {
    child: Child,
    pub port: u16,
    /// Kept open: kubectl exits on SIGPIPE when it logs "Handling connection" to a closed pipe.
    _stdout: BufReader<ChildStdout>,
}

impl PortForward {
    pub fn start(context: &str, namespace: &str, pod: &str, remote_port: u16) -> Self {
        let mut child = Command::new("kubectl")
            .args(["--context", context, "-n", namespace, "port-forward"])
            .arg(format!("pod/{pod}"))
            .arg(format!(":{remote_port}"))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("kubectl port-forward");
        let mut stdout = BufReader::new(child.stdout.take().expect("stdout is piped"));
        let mut line = String::new();
        stdout.read_line(&mut line).expect("port-forward output");
        // "Forwarding from 127.0.0.1:54321 -> 9090"
        let port = line
            .split("127.0.0.1:")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("unexpected port-forward output: {line}"));
        Self {
            child,
            port,
            _stdout: stdout,
        }
    }
}

impl Drop for PortForward {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
