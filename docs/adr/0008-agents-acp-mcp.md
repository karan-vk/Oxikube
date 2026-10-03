# ADR 0008: Oxikube is an ACP client hosting agents and an MCP server exposing cluster tools

- **Status:** Accepted (2026-10-03)
- **Deciders:** project owner, planning session
- **Related:** docs/PLAN.md decision table; docs/research/

## Context

The user wants Claude Code, Codex, Antigravity and Gemini inside the app as the final phase, with the architecture ready from day one. Zed hosts agents over ACP (agent-client-protocol 2.2, protocol v1); the ACP registry lists 41 agents incl. an official Antigravity binary adapter.

## Decision

`oxikube_acp` implements the ACP client (session/update, request_permission, fs/*, terminal/*, elicitation) and launches agents from the registry.json (schema-validated, cached) or built-in presets via npx/uvx/binary with checksums. `oxikube_mcp` (rmcp) exposes `oxikube_app::ToolRegistry` (k8s.*, helm.*, argo.*, app.*) over stdio to those agents; mutating tools are gated by MutationGuard + permission prompts; app.* tools dispatch Commands so agents can drive the UI. `ContextRegistry` resolves @-mentions and 'send to agent' payloads. fs/write_text_file on virtual oxikube:// paths becomes an editor diff that applies only after confirmation.

## Consequences

Every feature epic registers tool stubs and context providers as it lands (so Phase 5 is wiring, not retrofitting). ACP SDK churn is isolated behind `AgentPort` with an exact pin. Proprietary adapters are fetched at runtime with a licence notice, never bundled.
