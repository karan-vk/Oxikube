# Table API fixtures (E04-S04)

Recorded from the kind cluster (`cargo xtask kind-up`, Kubernetes v1.37) with
`Accept: application/json;as=Table;v=v1;g=meta.k8s.io,application/json` and
`includeObject=Metadata`; `managedFields` and the last-applied annotation are removed.

| File | Request |
|---|---|
| `widgets.json` | `GET /apis/test.oxikube.dev/v1/namespaces/oxikube-fixtures/widgets` (CRD with `additionalPrinterColumns`, one `priority: 1`) |
| `pods.json` | `GET /api/v1/namespaces/oxikube-fixtures/pods?labelSelector=app=fixtures-web` |
| `widgets-plain-list.json` | the same widgets with plain `Accept: application/json`: what a server that ignores the Table header sends |
