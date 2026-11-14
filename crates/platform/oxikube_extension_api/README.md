# oxikube_extension_api

**Layer:** `platform`

The SDK Oxikube extensions are written against. An extension is a WASM component, built for
`wasm32-wasip2`, implementing the WIT world `oxikube:extension/extension@0.1.0`:

- `wit/since_v0.1.0/` — the world (`extension.wit`) and its interfaces: `common`, `http-client`,
  `process`, `platform`, `settings` (with `key-value-store`) and `kube-info`. Released folders are
  frozen; breaking changes wait in `PENDING_CHANGES.md` for the next `since_vX` folder.
- `Extension` trait (`new` on `init-extension`, `run_command`, `context_server_command`) and the
  `register_extension!` macro that exports it.
- Host services re-exported as Rust modules (`settings::get`, `http_client::download_file`,
  `process::exec`, `key_value_store::insert`, `kube_info::contexts`, ...). A call without a grant
  fails with `Error::Denied(capability)`.

Extensions contribute commands, MCP context servers and (through `extension.toml`) themes. They
never contribute UI (ADR 0007).

```
cargo build -p oxikube_extension_api --target wasm32-wasip2 --examples   # examples/hello.rs as a component
cargo test -p oxikube_extension_api                                      # WIT parse + dispatch tests
```

An extension crate depends on this one, sets `crate-type = ["cdylib"]` and builds with
`cargo build --target wasm32-wasip2`.

## Allowed internal dependencies

- (none)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.
