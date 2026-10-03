# oxikube_domain

**Layer:** `domain`

Core with no internal dependencies and no I/O. Modules: `ids` (ClusterId, ContextName, Gvk/Gvr, Scope, ResourceRef), `kinds` (ResourceKind, Verb), `resource` (thin Resource: metadata + raw JSON), `view` (typed view-models), `quantity` + `age`, `session` (state machine, NamespaceSelection, WatchScope), `command` (Command, CommandId, CommandMeta, Capability), `safety` (Risk, ConfirmTier, Initiator), `audit` (AuditRecord), `log` / `event` / `metrics` (LogLine, Event, MetricsSample), `agent` (ContextBlock), `error` (OxiError, ErrorKind). Redaction has no module yet (planned: E19-S10). The glossary is `docs/CONTEXT.md`; `#![deny(missing_docs)]` is on.

## Allowed external dependencies

Pure data crates only: `serde`, `serde_json`, `serde-saphyr`, `thiserror`, `jiff`, `sha2`, `hex`, `bitflags`, `indexmap`, `smallvec`, `semver`, `url`, `regex` (and `proptest`/`insta` as dev-dependencies). Anything that does I/O, spawns tasks or talks to a cluster is banned (`cargo xtask lint-deps` enforces the ban list).

## Allowed internal dependencies

- (none)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
