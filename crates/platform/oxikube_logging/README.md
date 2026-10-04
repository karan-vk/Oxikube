# oxikube_logging

**Layer:** `platform`

tracing setup, rolling file logs, secret redaction layer, crash log hook.

Today: `RedactingMakeWriter` / `RedactingWriter` (scrub every formatted event with
`oxikube_domain::redact`), `RedactingFields` (redact secret-named fields), `redacting_layer` /
`redacting_json_layer`, and the shipped `default_filter` (info; HTTP stack at warn). The pattern
list and the rule that new secret-bearing fields must be added to it live in the module docs of
`oxikube_domain::redact`.

## Allowed internal dependencies

- `oxikube_domain`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
