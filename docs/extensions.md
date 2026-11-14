# Writing Oxikube extensions

An extension is a directory with an `extension.toml` and, when it has code, an `extension.wasm`
component built against `oxikube_extension_api`. Design: ADR 0007 (Zed's extension model on
wasmtime + WIT).

## What extensions can and cannot do

Extensions can contribute:

- themes and icon themes (plain JSON files, no code),
- commands, shown in the command palette,
- MCP context servers (the extension says how to start the server; the host runs it).

Extensions **cannot add UI**: no views, panels, menus or tabs. Custom views are not supported.
For first-class UI over another system use an integration (ADR 0009).

## Quick start

```
cargo xtask ext new my-ext                  # a palette command (default), written to ./my-ext
cargo xtask ext new my-server --kind mcp    # an MCP context server
cargo xtask ext new my-theme --kind theme   # a theme, no code
cd my-ext && ./build.sh
```

Inside this repository the generated `Cargo.toml` points at the SDK with a relative path; elsewhere
it points at the absolute path of the checkout that ran xtask (the SDK is not published).
`build.sh` runs `cargo build --release --target wasm32-wasip2` and copies the component to
`extension.wasm`. The target is installed by the repo's `rust-toolchain.toml`; elsewhere run
`rustup target add wasm32-wasip2`.

Then run "Install Dev Extension" from the command palette and pick the directory.

## Manifest reference (`extension.toml`)

```toml
id = "my-ext"            # [a-z0-9-]+, unique
name = "My Extension"
version = "0.1.0"
schema_version = 1       # the only supported value
description = "One line"
authors = ["Me <me@example.com>"]
repository = "https://github.com/me/my-ext"
themes = ["themes/dark.json"]          # relative to the extension directory
icon_themes = ["icons/icons.json"]
```

Code-bearing extensions add a `[lib]` table, their commands and their context servers:

```toml
[lib]
kind = "rust"
version = "0.1.0"        # oxikube_extension_api version it was built against

[[commands]]
id = "say-hello"         # plain id, no ':'
title = "Say Hello"
description = "Shown in the palette"
scopes = ["global"]      # where the palette offers it; defaults to global

[[context_servers]]
id = "my-server"
```

Unknown fields are rejected at install time, with the field name in the error.

## Capabilities

Host calls beyond pure computation need a grant. The manifest declares what the extension may ask
for and the user grants it at install time. A call without a grant fails with `Error::Denied`.

```toml
capabilities = [
  { kind = "process:exec", command = "kubectl", args = ["get", "*"] },
  { kind = "download_file", host = "github.com", path = "/my-org/*" },
  { kind = "kube:read", contexts = ["kind-*"] },
]
```

Patterns are globs: `*` matches any run of characters (including none), every other character
matches itself. An empty pattern is rejected. `kube:read` never exposes credentials and never
mutates.

## The extension API

```rust
use oxikube_extension_api::{self as oxikube, Extension};

struct MyExt;

impl Extension for MyExt {
    fn new() -> Self { Self }

    fn run_command(&mut self, command_id: &str, args: Vec<String>) -> Result<(), String> {
        let _ = args;
        Err(format!("unknown command `{command_id}`"))
    }
}

oxikube::register_extension!(MyExt);
```

`run_command` handles the ids in `[[commands]]`; `context_server_command` returns how to start an
id from `[[context_servers]]`. Host services are the modules `settings`, `key_value_store`,
`http_client`, `process`, `platform` and `kube_info`.

## WIT version policy

The WIT world lives in `crates/platform/oxikube_extension_api/wit/since_vX.Y.Z/`. A released
folder is frozen: an extension built against it must keep loading. Breaking changes wait in
`PENDING_CHANGES.md` and land together in the next `since_vX` folder, which the host supports
next to the older ones. `[lib] version` records which API version the component targets.

## Debugging

- Run the app from a terminal to see the host's log (`RUST_LOG=oxikube_extension_host=debug`).
- A command's `Err(String)` is shown to the user; use it for failures you expect.
- The host interrupts guest code that runs too long (epoch interruption) and reports a timeout;
  keep commands short and do slow work in a spawned process.
- A trap or a missing grant surfaces as an error naming the extension and the capability.

## Security model

Extensions run in a wasmtime sandbox with no ambient access: no filesystem outside their working
directory, no network, no processes, no credentials. Every escape hatch is a host call checked
against the user's grants. Secrets must never be written to logs or disk by an extension.

## Samples

`extensions/` holds sample extensions (a theme, a command, an MCP server): `theme-sample`,
`hello-command`, `mcp-echo`.
