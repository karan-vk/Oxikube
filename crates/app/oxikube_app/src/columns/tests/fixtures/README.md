# Recorded Table responses (E07-S02)

Copies of `crates/adapters/oxikube_kube/tests/fixtures/table/{widgets,pods}.json`, recorded from
the kind cluster with `Accept: application/json;as=Table;v=v1;g=meta.k8s.io,application/json` and
`includeObject=Metadata`. `widgets.json` is a CRD with `additionalPrinterColumns` (one at
`priority: 1`); `pods.json` is the pod table, whose `Ready`, `Restarts` and `Age` columns are all
typed `string`.
