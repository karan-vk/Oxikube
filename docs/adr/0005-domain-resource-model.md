# ADR 0005: Domain owns a thin Resource model; k8s-openapi only in the kube adapter

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Kubernetes has 200+ kinds plus arbitrary CRDs. Re-modelling them in the domain is anti-KISS; depending on k8s-openapi in the domain leaks the adapter's data model and versioning.

## Decision

`oxikube_domain::Resource { meta: ObjectMeta, kind: Gvk, json: serde_json::Value }` with JSON-pointer accessors. Typed view-models (PodSummary, NodeSummary, WorkloadSummary, …) are built from JSON in the domain for core kinds. Ports speak Gvk/Gvr + Resource/Value. k8s-openapi types appear only inside `oxikube_kube` (and kopium-generated Argo types inside `oxikube_argocd`).

## Consequences

CRDs and unknown kinds work for free. Typed projections may show missing fields on very old/new servers; discovery decides what exists. Mapping code lives in one adapter.
