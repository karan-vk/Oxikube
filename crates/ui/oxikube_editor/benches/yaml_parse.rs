//! Benchmark (E10-S02; E10-S11 reuses it): `yaml::parse` over a 2k-line multi-document manifest,
//! the same with a syntax error in the middle (recovery path), a 5 MB `kubectl get -o yaml`
//! style dump, and 5 MB buffers with hundreds of errors (recovery stays linear).
//!
//! Every case asserts that the whole buffer reached the model (the last line's key is in the
//! tree), so a case that hits the parser's diagnostic cap fails instead of timing a prefix.
//!
//! `cargo bench -p oxikube_editor --bench yaml_parse` prints min / median / p95 per parse. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one small iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::fmt::Write as _;
use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_editor::yaml::parse_shared;

fn deployment(i: usize) -> String {
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
fn manifest(lines: usize) -> String {
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
fn dump(bytes: usize) -> String {
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

/// Offset of the key on the last line of `text` (after its indentation and `- ` markers).
fn last_line_key(text: &str) -> usize {
    let body = text.trim_end_matches('\n');
    let start = body.rfind('\n').map_or(0, |i| i + 1);
    let line = &body[start..];
    start + line.len() - line.trim_start_matches([' ', '-']).len()
}

fn bench(label: &str, text: &str, samples: usize) {
    let text: Arc<str> = Arc::from(text);
    let first = parse_shared(Arc::clone(&text));
    assert!(
        first.key_at(last_line_key(&text)).is_some(),
        "{label}: the last line is not in the model ({} diagnostics): recovery stopped early, \
         so the timing would cover only a prefix of the buffer",
        first.diagnostics().len(),
    );
    let nodes: usize = first.docs().iter().map(|d| d.nodes().len()).sum();
    let mut times: Vec<Duration> = (0..samples)
        .map(|_| {
            let start = Instant::now();
            black_box(parse_shared(Arc::clone(&text)));
            start.elapsed()
        })
        .collect();
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<28} {:>5} lines {:>8} B {:>7} nodes {:>2} diags  min {:>9.2?}  median {:>9.2?}  p95 {:>9.2?}",
        text.lines().count(),
        text.len(),
        nodes,
        first.diagnostics().len(),
        at(0.0),
        at(0.5),
        at(0.95),
    );
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let (samples, big) = if smoke {
        (1, 64 * 1024)
    } else {
        (50, 5 * 1024 * 1024)
    };

    let small = manifest(2_000);
    bench("2k-line manifest", &small, samples * 4);

    let mid = small.len() / 2;
    let cut = small[mid..].find('\n').map_or(mid, |i| mid + i + 1);
    let broken = format!(
        "{}  broken: \"unterminated\n{}",
        &small[..cut],
        &small[cut..]
    );
    bench("2k-line manifest, 1 error", &broken, samples * 4);

    let dump = dump(big);
    bench("5 MB get -o yaml dump", &dump, samples.min(10));

    // Recovery must stay linear in the buffer size however many errors it hits.
    let mut pods = 0;
    let mut many = String::with_capacity(dump.len() + dump.len() / 1_000);
    for line in dump.split_inclusive('\n') {
        many.push_str(line);
        if line.starts_with("- apiVersion") {
            pods += 1;
            if pods % 25 == 0 {
                many.push_str("  bad: x: y\n");
            }
        }
    }
    bench("5 MB dump, error / 25 pods", &many, samples.min(10));

    // One error per 1000 lines: ~490 at 5 MB, under the parser's 1000-diagnostic cap, so the
    // whole buffer is parsed (`bench` asserts it).
    let mut flat = String::with_capacity(big + 64);
    for i in 0.. {
        if flat.len() >= big {
            break;
        }
        if i % 1_000 == 999 {
            // A unique key, so each error line adds one syntax diagnostic and no duplicate key.
            let _ = writeln!(flat, "e{i}: b: c");
        } else {
            let _ = writeln!(flat, "k{i}: v");
        }
    }
    flat.push_str("last: v\n");
    bench("5 MB flat, error / 1000 lines", &flat, samples.min(10));
}
