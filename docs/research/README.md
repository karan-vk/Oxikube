# Research reports (2026-10-03)

Reports gathered during the planning session that produced the epic catalogue. They are reference material for agent teams; the decisions derived from them live in `docs/adr/` and the plan in `docs/PLAN.md`. All reports were produced read-only from public sources; items marked "not verified" need a spike before being relied on.

| Report | What it covers |
|---|---|
| [features-freelens-k9s.md](features-freelens-k9s.md) | Exhaustive feature inventory of Freelens/OpenLens/Lens and k9s: every resource view with its columns and actions, cross-cutting features (auth, multi-cluster, logs, exec, port-forward, metrics, Helm, CRDs, RBAC, extensions, settings, themes, keybindings), condensed k9s command/key reference, user complaints and gaps mined from GitHub issues, notes on Headlamp/Aptakube/Seabird/kubetui, and the prior GPUI Kubernetes clients (Kubyl, Periscope, baeus). Source of the resource catalog in E07/E12. |
| [kdash-and-rust-k8s-ecosystem.md](kdash-and-rust-k8s-ecosystem.md) | kdash (MIT) architecture and a file:function map of what is worth reusing (kubeconfig loader, cronjob trigger, patch builders, log reconnect logic) versus what to avoid (polling architecture, quantity parsing, pod status derivation). kube-rs 4.2 capability matrix with feature flags, the hand-rolled Table API workaround, k8s-metrics, deskribe, Helm options, YAML/diff/terminal/plugin/ACP crates, and other Rust Kubernetes UIs with licences. |
| [gpui-and-zed-internals.md](gpui-and-zed-internals.md) | How to depend on GPUI (gpui-pre snapshots vs Zed git), gpui-component contents and pitfalls, GPUI core concepts, Zed workspace conventions and init order, Zed's theme/settings/extension/ACP/terminal/editor designs, lessons from Kubyl and Periscope handoff logs, recommendations and risks, plus the Zed v1.22.0 internal-dependency table (which crates drag in `project`/`client`/`editor`). |
| [argocd-integration.md](argocd-integration.md) | Argo CD v3.5 feature → UI → REST → CLI → CRD-direct table, API/transport/auth details (SSE vs NDJSON streams, cookie-authed terminal WebSocket, PKCE loopback SSO), what CRD-direct can and cannot do, how `argocd --core` really works, Rust client options, how other clients integrate, Argo Rollouts patch recipes, the three-backend recommendation and risks. |

## Conclusions that became ADRs
- **License GPL-3.0-or-later** (Zed's model): permits copying Zed GPL code with headers; kdash (MIT), gpui-component (Apache-2.0), deskribe (Apache-2.0 + Kubernetes NOTICE) remain compatible.
- **GPUI via `gpui-pre` pinned exactly to gpui-component's pin** (0.3.7 ↔ zed@1a28cff, gpui-component 0.7.0); a Zed git dependency cannot coexist with gpui-component, and Zed's shell crates (workspace, editor, terminal_view, agent_ui) drag in 90–154 internal crates, so they are copied as design only.
- **gpui-component behind a thin `oxikube_ui` wrapper** for table/dock/editor/charts/markdown; own terminal element on `alacritty_terminal` 0.26.
- **Hybrid table data**: typed/metadata reflectors with our own column definitions for core kinds; hand-rolled server Table API feed for CRDs and unknown kinds (kube-rs has no Table support).
- **Thin domain Resource model** (GVK + metadata + raw JSON) with k8s-openapi confined to the kube adapter; `k8s-metrics` behind MetricsPort; `deskribe` behind DescribePort with kubectl fallback.
- **Helm**: native release-Secret decoding for reads, `helm` CLI for mutations; no Rust Helm engine exists.
- **Argo CD**: three backends (CRD-direct via kube + kopium types, hand-written REST client, `argocd admin dashboard` core helper) behind one `ArgoBackend` trait with capability-gated UI; Rollouts fully CRD-direct.
- **Extensions**: wasmtime component model + WIT `since_vX` worlds + extension.toml capability grants + epoch interruption, exactly Zed's design, with no UI hooks.
- **ACP**: `agent-client-protocol =2.2.0` client; registry.json fetched at runtime with built-in presets (Claude Code, Codex, Antigravity binary adapter, Gemini); proprietary adapters are fetched, never bundled.
- **Reuse from kdash limited to small pure functions** with MIT attribution; never its polling architecture.
