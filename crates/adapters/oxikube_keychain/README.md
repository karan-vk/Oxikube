# oxikube_keychain

**Layer:** `adapters`

SecretStorePort adapter on the keyring crate (macOS Keychain, Linux Secret Service, Windows Credential Manager).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

See `docs/ROADMAP.md` and the GitHub Project for the epics and stories that build this crate.
