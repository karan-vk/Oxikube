# Native release reader fixtures

`helm4.3.1-<kind>-<release>.v<rev>.json` are release objects recorded from a real
`helm` v4.3.1 against the kind cluster (`kubectl get -o json`, `managedFields` removed), in
namespace `oxi-test-helmcap12395`, from the chart in `chart/`:

```
helm install web chart --set greeting=hi                      # web v1 (superseded)
helm upgrade web chart --set image.tag=1.1 --set ports=null    # web v2 (deployed)
helm upgrade web chart --reuse-values --set fail=true          # web v3 (failed at apply)
HELM_DRIVER=configmap helm install web.cm-app chart            # web.cm-app v1 (superseded)
HELM_DRIVER=configmap helm upgrade web.cm-app chart --set greeting=yo   # v2 (deployed)
helm install gone chart && helm uninstall gone --keep-history  # gone v1 (uninstalled)
```

The chart's values hold no credentials. Re-record with the same commands into a fresh
`oxi-test-*` namespace if Helm's storage format changes; name the files after the Helm
version. Synthetic records (pending states, large manifests, decode failures) are built in
`tests/native/support.rs` instead.
