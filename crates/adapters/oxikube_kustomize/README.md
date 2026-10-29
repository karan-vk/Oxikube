# oxikube_kustomize

**Layer:** `adapters`

`KustomizePort` adapter (E20-S04): detects the user's `kustomize` (or `kubectl kustomize`) and builds overlay directories with it. Shell-out only; no exec plugins, alpha plugins or Helm; stdout is never logged.

## Modules

- `config`: `KustomizeConfig` (the `kustomize.path` override, build and probe deadlines, where to search).
- `path`: the login shell's `PATH` and the binary lookup.
- `detect`: standalone first, then `kubectl kustomize`; version parsing.
- `run`: one child process, separate stdout and stderr, deadline, killed on drop.
- `build`: the build command line and failure classification.
- `port`: `KustomizeCli`, the `KustomizePort` implementation (cached detection, `reconfigure`).

Tests: unit tests per module and `tests/stub_binary.rs` (stub executables on a controlled `PATH`; one test uses a real kustomize when installed and skips otherwise).
