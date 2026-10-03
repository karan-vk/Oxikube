# oxikube_cloud

**Layer:** `adapters`

CloudDiscoveryPort adapters that shell out to installed aws/gcloud/az CLIs to list EKS/GKE/AKS clusters and build exec-auth kubeconfig entries.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
