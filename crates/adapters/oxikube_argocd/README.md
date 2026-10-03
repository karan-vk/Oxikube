# oxikube_argocd

**Layer:** `adapters`

Argo CD integration adapters: CRD-direct backend (kopium types), argocd-server REST backend (SSE/NDJSON streams, cookie-auth terminal WS, token/CLI-config/SSO PKCE auth), core helper (argocd admin dashboard), Argo Rollouts CRD actions.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
