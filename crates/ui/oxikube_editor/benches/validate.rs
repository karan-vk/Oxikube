//! Benchmark (E10-S03; E10-S11 reuses it): schema validation of a 2k-line multi-document
//! Deployment manifest, clean and with one mistake per document, and the same with the parse
//! included (what a keystroke costs when the parse is not cached). Budget: under 50 ms on 2k lines
//! (the epic's goal is squiggles within 100 ms of typing).
//!
//! `cargo bench -p oxikube_editor --bench validate` prints min / median / p95. Under
//! `cargo test --all-targets` (no `--bench` flag) it runs one iteration as a smoke test.

#![allow(clippy::print_stdout, reason = "a benchmark reports on stdout")]

use std::hint::black_box;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oxikube_domain::ids::Gvk;
use oxikube_domain::schema::{JsonSchema, root_schema_for};
use oxikube_editor::validate::{ValidateOptions, validate_buffer};
use oxikube_editor::yaml::parse_shared;
use serde_json::json;

/// A Deployment schema as large as the parts of the real one this manifest touches.
fn deployment_schema() -> Arc<JsonSchema> {
    let quantity = json!({"oneOf": [{"type": "string"}, {"type": "number"}]});
    let strings = json!({"type": "array", "items": {"type": "string"}});
    let document = json!({"components": {"schemas": {
        "Deployment": {
            "type": "object",
            "required": ["spec"],
            "properties": {
                "apiVersion": {"type": "string"},
                "kind": {"type": "string"},
                "metadata": {"$ref": "#/components/schemas/ObjectMeta"},
                "spec": {"$ref": "#/components/schemas/DeploymentSpec"},
                "status": {"type": "object"},
            },
            "x-kubernetes-group-version-kind": [
                {"group": "apps", "kind": "Deployment", "version": "v1"}
            ],
        },
        "ObjectMeta": {"type": "object", "properties": {
            "name": {"type": "string"},
            "namespace": {"type": "string"},
            "labels": {"type": "object", "additionalProperties": {"type": "string"}},
            "annotations": {"type": "object", "additionalProperties": {"type": "string"}},
        }},
        "DeploymentSpec": {"type": "object", "required": ["selector", "template"], "properties": {
            "replicas": {"type": "integer"},
            "selector": {"type": "object", "required": ["matchLabels"], "properties": {
                "matchLabels": {"type": "object", "additionalProperties": {"type": "string"}},
            }},
            "template": {"type": "object", "properties": {
                "metadata": {"$ref": "#/components/schemas/ObjectMeta"},
                "spec": {"$ref": "#/components/schemas/PodSpec"},
            }},
        }},
        "PodSpec": {"type": "object", "required": ["containers"], "properties": {
            "containers": {
                "type": "array",
                "items": {"$ref": "#/components/schemas/Container"},
                "x-kubernetes-list-type": "map",
                "x-kubernetes-list-map-keys": ["name"],
            },
        }},
        "Container": {"type": "object", "required": ["name"], "properties": {
            "name": {"type": "string", "pattern": "^[a-z0-9]([-a-z0-9]*[a-z0-9])?$"},
            "image": {"type": "string"},
            "args": strings,
            "command": strings,
            "ports": {
                "type": "array",
                "items": {"type": "object", "required": ["containerPort"], "properties": {
                    "containerPort": {"type": "integer"},
                    "protocol": {"type": "string", "enum": ["SCTP", "TCP", "UDP"]},
                }},
                "x-kubernetes-list-type": "map",
                "x-kubernetes-list-map-keys": ["containerPort", "protocol"],
            },
            "env": {
                "type": "array",
                "items": {"type": "object", "required": ["name"], "properties": {
                    "name": {"type": "string"},
                    "value": {"type": "string"},
                }},
                "x-kubernetes-list-type": "map",
                "x-kubernetes-list-map-keys": ["name"],
            },
            "resources": {"type": "object", "properties": {
                "limits": {"type": "object", "additionalProperties": quantity},
                "requests": {"type": "object", "additionalProperties": quantity},
            }},
        }},
    }}});
    let gvk = Gvk::new("apps", "v1", "Deployment");
    Arc::new(root_schema_for(&document, &gvk).expect("root schema"))
}

/// One Deployment; `broken` adds an unknown field, a string replica count and a bad protocol.
fn deployment(i: usize, broken: bool) -> String {
    let (replicas, extra, protocol) = if broken {
        ("\"3\"", "  replica: 2\n", "TCPP")
    } else {
        ("3", "", "TCP")
    };
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
  replicas: {replicas}
{extra}  selector:
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
              protocol: {protocol}
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
fn manifest(lines: usize, broken: bool) -> String {
    let mut out = String::new();
    let mut i = 0;
    while out.lines().count() < lines {
        if i > 0 {
            out.push_str("---\n");
        }
        out.push_str(&deployment(i, broken));
        i += 1;
    }
    out
}

fn report(label: &str, lines: usize, diags: usize, mut times: Vec<Duration>) -> Duration {
    times.sort();
    let at = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "{label:<36} {lines:>5} lines {diags:>4} diags  min {:>9.2?}  median {:>9.2?}  p95 {:>9.2?}",
        at(0.0),
        at(0.5),
        at(0.95),
    );
    at(0.95)
}

fn bench(
    label: &str,
    text: &str,
    schema: &Arc<JsonSchema>,
    samples: usize,
    with_parse: bool,
) -> Duration {
    let text: Arc<str> = Arc::from(text);
    let opts = ValidateOptions::default();
    let parsed = parse_shared(Arc::clone(&text));
    let run = |parsed: &_| validate_buffer(parsed, |_| Some(Arc::clone(schema)), &opts);
    let diags = run(&parsed).len();
    let times = (0..samples)
        .map(|_| {
            let start = Instant::now();
            if with_parse {
                let parsed = parse_shared(Arc::clone(&text));
                black_box(run(&parsed));
            } else {
                black_box(run(&parsed));
            }
            start.elapsed()
        })
        .collect();
    report(label, text.lines().count(), diags, times)
}

fn main() {
    let smoke = !std::env::args().any(|a| a == "--bench");
    let samples = if smoke { 1 } else { 200 };
    let schema = deployment_schema();

    let clean = manifest(2_000, false);
    let mistakes = manifest(2_000, true);
    let p95s = [
        bench("validate, 2k lines, valid", &clean, &schema, samples, false),
        bench(
            "validate, 2k lines, 3 faults/doc",
            &mistakes,
            &schema,
            samples,
            false,
        ),
        bench(
            "parse + validate, 2k lines, valid",
            &clean,
            &schema,
            samples,
            true,
        ),
        bench(
            "parse + validate, 2k lines, faults",
            &mistakes,
            &schema,
            samples,
            true,
        ),
    ];
    if !smoke {
        let budget = Duration::from_millis(50);
        assert!(
            p95s.iter().all(|p| *p < budget),
            "over the {budget:?} budget: {p95s:?}"
        );
    }

    if smoke {
        let parsed = parse_shared(Arc::from(clean.as_str()));
        let diags = validate_buffer(
            &parsed,
            |_| Some(Arc::clone(&schema)),
            &ValidateOptions::default(),
        );
        assert!(
            diags.is_empty(),
            "a valid manifest has no diagnostics: {diags:?}"
        );
    }
}
