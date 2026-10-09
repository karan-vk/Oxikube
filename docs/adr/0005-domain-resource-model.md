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

## Amendment: the JSON is a compact document, not a tree (E07-P603)

The shared `Arc<Value>` of the previous amendment still cost 8 to 17 KB of small allocations per pod (one node per field, a `String` per key and text, a table per object), which is about 10 KB per pod across the app's caches, indices and tables and put the idle app (two clusters of 1 000 pods) 43 MiB over its memory budget.

`Resource` now holds `doc: JsonDoc` (`oxikube_domain::json`): the object's JSON as one immutable byte buffer, shared between clones, in a tagged varint format in which the common Kubernetes keys are one byte (`json::keys`) and containers carry their byte length so a lookup skips whole subtrees. Reads go through `JsonRef`, a `Copy` borrowed view with the read half of `Value` (`get`, `pointer`, `as_str`, `as_array`, ...); it borrows strings from the buffer and never allocates. `Resource::to_value()` builds a `Value` tree for cold paths (the editor, CRD schemas, `Event::from_json`), `edit_json` decodes, edits and re-encodes, and `to_yaml` serialises the document directly. The ports are unchanged (`Resource` is still what they carry), key order is kept for the YAML view, and `==` is JSON equality as `Value`'s is.

`ObjectMeta` shrank in the same story: `labels` and `annotations` are `StrMap`s (a sorted `Arc<[(key, value)]>`; label sets equal across pods are one allocation) instead of `BTreeMap`s, and the texts many objects repeat (namespace, label keys and values, owner kinds, the `Gvk`) are `intern`ed: one `Arc<str>` per distinct text while an object uses it. Measured in `oxikube_testkit`'s `heap_per_pod` test: a pod is 8.4 KB in 122 blocks as a `Value` tree and 0.9 KB in 3 blocks as a whole `Resource`; numbers for the app in docs/PERFORMANCE.md ("Memory: compact object storage").
