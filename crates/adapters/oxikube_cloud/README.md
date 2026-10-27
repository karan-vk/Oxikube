# oxikube_cloud

**Layer:** `adapters`

CloudDiscoveryPort adapters that shell out to installed aws/gcloud/az CLIs to list EKS/GKE/AKS clusters and build exec-auth kubeconfig entries.

## Modules

- `process` (E18-S01): `CliRunner` runs `aws`, `gcloud`, `az`, `gke-gcloud-auth-plugin` and
  `kubelogin` with a timeout (30 s default), a `CancellationToken`, JSON output flags
  (`run_json::<T>`), a concurrency cap, the login shell's `PATH` and per-CLI path overrides.
  Failures are a typed `CliError` with redacted stderr, mapped onto `OxiError`. Provider
  adapters take an `Arc<dyn ProcessRunner>`.
- `testing` (feature `testing`): `RecordedRunner` answers commands from recorded output
  (`tests/fixtures/<cli>-<version>/...`), so adapters are tested without any cloud account.

The runner's design mirrors `oxikube_helm::cli::run` (adapters never depend on each other).
Nothing secret goes on a command line, into the environment the runner sets or into a log
line: logs carry the command label (`aws eks list-clusters`) and its outcome only.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
