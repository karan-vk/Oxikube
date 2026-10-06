# oxikube_logging

**Layer:** `platform`

tracing setup, rolling file logs, secret redaction layer, crash log hook.

`init` / `build` set up the global subscriber (E05-S09): a reloadable `RUST_LOG`-style filter, a
redacting layer over daily rolling files (`LogConfig`, seven kept, non-blocking writer, `log` crate
bridged), `LogHandle` to change the filter live and the `log.filter` setting (`follow`).
`install_panic_hook` writes a redacted crash report file and then calls the previous hook.

Also: `RedactingMakeWriter` / `RedactingWriter` (scrub every formatted event with
`oxikube_domain::redact`), `RedactingFields` (redact secret-named fields), `redacting_layer` /
`redacting_json_layer`, and the shipped `default_filter` (info; HTTP stack at warn). The pattern
list and the rule that new secret-bearing fields must be added to it live in the module docs of
`oxikube_domain::redact`.

## Allowed internal dependencies

- `oxikube_domain`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
