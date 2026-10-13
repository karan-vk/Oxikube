# oxikube_notify_os

**Layer:** `adapters`

`NotifierPort` adapter for OS desktop notifications (E14-S06): `OsNotifier`, off until the user opts
in with `events.os_notifications`.

| Platform | Backend |
|---|---|
| macOS | `NSUserNotificationCenter` through `mac-notification-sys` (click reported via `wait_for_click`) |
| Linux, BSDs | freedesktop notifications over D-Bus through `notify-rust` (zbus); click via the `default` action when the server has the `actions` capability |
| other | always `Unavailable` |

Nothing contacts the OS until `set_enabled(true)`. A machine with no notification service answers
`NotifierStatus::Unavailable` and `notify` returns the `oxikube_ports::unavailable` error; nothing panics.

Manual check (shows one notification): `OXIKUBE_TEST_OS_NOTIFY=1 cargo test -p oxikube_notify_os
--test degrades_gracefully -- --ignored`.

macOS caveat: a binary outside an `.app` bundle is attributed to Terminal, may be suppressed by the OS and may not
get its click back; use a bundled build to test click-through.

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

## Owning epics

E14 (Events & notifications), story E14-S06.
