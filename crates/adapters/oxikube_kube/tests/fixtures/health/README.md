# Health fixtures

Review answers recorded from the kind test cluster (`kind-oxikube`), impersonating the
`kube-system:coredns` service account so the rules are small and realistic:

```
kubectl --context kind-oxikube --as system:serviceaccount:kube-system:coredns \
  create --validate=false -o json -f <review.json>
```

- `selfsubjectrulesreview.json`: `SelfSubjectRulesReview` for namespace `default`
  (list/watch on core objects, discovery, review rights; no mutate, exec, logs or
  port-forward).
- `selfsubjectaccessreview.json`: `SelfSubjectAccessReview` for `create pods/exec` in
  `default` (denied, no reason).

Used by the fake-HTTP unit tests in `src/health/tests.rs`.
