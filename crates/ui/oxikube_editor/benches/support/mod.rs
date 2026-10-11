//! Generated manifests shared by the editor's benchmarks.

#![allow(dead_code, reason = "each benchmark uses a subset")]

use std::fmt::Write as _;

pub fn deployment(i: usize) -> String {
    format!(
        r#"apiVersion: apps/v1
kind: Deployment
metadata:
  name: web-{i}
  namespace: default
  labels:
    app.kubernetes.io/name: web-{i}  # selector label
    tier: "frontend"
spec:
  replicas: 3
  selector:
    matchLabels: {{app.kubernetes.io/name: web-{i}}}
  template:
    metadata:
      labels:
        app.kubernetes.io/name: web-{i}
    spec:
      containers:
        - name: nginx
          image: nginx:1.27
          args: ["--port", "8080"]
          ports:
            - containerPort: 8080
              protocol: TCP
          env:
            - name: GREETING
              value: 'hello, world'
          command:
            - sh
            - -c
            - |
              echo starting
              exec nginx -g 'daemon off;'
          resources:
            limits: {{cpu: 500m, memory: 128Mi}}
"#
    )
}

/// About `lines` lines of `---`-separated Deployments.
pub fn manifest(lines: usize) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.lines().count() < lines {
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&deployment(i));
        i += 1;
    }
    out
}

/// About `bytes` of a `kubectl get pods -o yaml` list.
pub fn dump(bytes: usize) -> String {
    let mut out = String::from("apiVersion: v1\nkind: List\nitems:\n");
    let mut i = 0;
    while out.len() < bytes {
        let _ = write!(
            out,
            r#"- apiVersion: v1
  kind: Pod
  metadata:
    name: pod-{i}
    namespace: ns-{n}
    uid: 6f1c{i:08}-aaaa-bbbb-cccc-0123456789ab
    labels: {{app: app-{n}, pod-template-hash: 5d8f7c{i}}}
    annotations:
      kubectl.kubernetes.io/restartedAt: "2026-01-02T03:04:05Z"
  spec:
    nodeName: worker-{n}
    containers:
    - name: main
      image: registry.example.com/app:{n}
      ports:
      - containerPort: 8080
  status:
    phase: Running
    podIP: 10.0.{n}.{m}
    conditions:
    - type: Ready
      status: "True"
"#,
            n = i % 50,
            m = i % 250,
        );
        i += 1;
    }
    out
}
