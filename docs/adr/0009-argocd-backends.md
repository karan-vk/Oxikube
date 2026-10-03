# ADR 0009: Argo CD via three capability-gated backends behind one trait, plus a generic IntegrationPort

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

Argo CD state is readable from CRDs (Application/ApplicationSet/AppProject) with no extra credentials, but diff, target manifests, repo browsing, Lua resource actions and account/token management need argocd-server. `argocd --core` is actually a local in-process server that port-forwards to repo-server and Redis. No maintained Rust client exists; the swagger is 2.0 with 270 recursive definitions (poor codegen fit).

## Decision

A generic `IntegrationPort` (detect → sidebar, commands, tools, context providers, settings) with Argo CD as the first implementation. `ArgoBackend` trait with capability flags and three backends: CRD-direct (kube + kopium types; list/watch/sync/refresh/rollback/terminate/delete/spec edits), Server REST (hand-written reqwest client for ~45 endpoints, NDJSON/SSE streams, cookie-authed terminal WS; auth = token entry, import ~/.config/argocd/config, keychain, native SSO PKCE loopback), Core helper (spawn `argocd admin dashboard --port N` when the binary exists). UI declares required capabilities and shows which backend served it. Argo Rollouts is CRD-direct (merge patches). Nothing appears unless the CRDs exist or a server profile is configured.

## Consequences

Argo CD web-UI parity without a mandatory server login; three code paths to test (fixtures for 3.3/3.4/3.5). Unverified protocol details (terminal WS ops, sync-window enforcement, SSE through proxies) are spike stories before dependent work. Flux follows the same IntegrationPort later.
