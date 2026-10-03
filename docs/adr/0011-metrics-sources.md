# ADR 0011: metrics-server always; pluggable Prometheus-compatible adapter for history

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Lens shows Prometheus graphs (and installs its own stack); k9s shows metrics-server point values. Users complain when either is mandatory. Nobody wants a client installing things into their cluster.

## Decision

`MetricsPort` on metrics.k8s.io via the k8s-metrics crate (with internal fallback types) feeds live table columns; `PromqlPort` with auto-detected providers (kube-prometheus, Lens stack, VictoriaMetrics, Mimir, OpenShift) or manual URL feeds history graphs. Absence is a visible state, never a swallowed error. We never install a metrics stack.

## Consequences

Graphs appear only when a PromQL endpoint is reachable; provider query sets need fixture tests per stack.
