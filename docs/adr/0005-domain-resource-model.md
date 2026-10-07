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

## Amendment: one shared JSON tree (#508)

`Resource::json` is an `Arc<serde_json::Value>`, so cloning a `Resource` shares the tree instead of copying it. A parsed `Value` costs about eight times its JSON (a small pause pod: 2 KB of JSON, 16 KB and 160 allocations as a tree), and every watched object used to be held twice: once in the kube adapter's reflector store and once in the resource store's cache, which received a clone. With the shared tree a clone costs only its `ObjectMeta` (about 1 KB), and 10 000 pods stay under the 400 MB budget (ADR 0013; numbers in docs/PERFORMANCE.md). Reads are unchanged (`&Value` through deref); writes go through `Resource::json_mut` (copy on write, so a clone never sees the change) and `Resource::into_json` takes the value out. Ports still speak `Resource`, so no port changed.
