# Roadmap

Phases map to GitHub milestones M0–M6 and the `Phase` field of the GitHub Project.
Full epic/story catalogue: `docs/PLAN.md`. Live status: the GitHub Project "Oxikube".

| Phase | Epics | Outcome |
|---|---|---|
| 0 Foundation | E01 Workspace & tooling · E02 Domain, ports & testkit · E03 Kube connection/auth/discovery · E04 Kube data plane · E05 App shell | Compiling, lint-enforced workspace; real cluster adapter proven on kind; Zed-like shell with settings/keymap/theme cores |
| 1 Core Kubernetes | E06 Cluster catalog & sessions · E07 Resource browser · E08 Logs · E09 Terminal & exec · E10 Manifest editor · E11 Palette, ':' jump & keymaps | **v0.1**: connect clusters, browse every kind incl. CRDs, logs, exec, edit YAML with schema validation and dry-run |
| 2 Lens parity | E12 Per-kind panels & actions · E13 Metrics & overview · E14 Events & notifications · E15 Port-forward manager · E16 Helm · E17 RBAC · E18 Cloud discovery · E19 Safety & audit · E20 Apply/kustomize, file transfer, cross-cluster | **v0.5**: everything Lens/OpenLens/k9s do, natively |
| 3 Zed-style platform | E21 Settings & keymap UI · E22 Theming · E23 Extensions · E24 Release engineering | **v0.8**: GUI settings, Zed themes, WASM extensions, signed releases + auto-update |
| 4 Integrations | E25 Integration framework + Argo CD + Rollouts | Argo CD web-UI parity, optional, capability-gated |
| 5 Agents | E26 Agent foundation (MCP tools, context) · E27 ACP client & agent panel | **v1.0**: Claude Code / Codex / Antigravity / Gemini hosted in-app with cluster tools and context |
| 6 Later | E28 Windows · E29 Backlog (Flux, extension registry, fleet views, a11y, scanning, …) | |

Designed-for from day one even though built last: every action registers an MCP tool stub and a
`Command`; every view can "send to agent"; ports for integrations/tools/context/agent are fixed in E02.
