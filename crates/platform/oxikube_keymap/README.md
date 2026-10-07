# oxikube_keymap

**Layer:** `platform`

Layered, hot-reloading key bindings in Zed's `keymap.json` format (E05-S07): per-OS defaults
(embedded in `oxikube_assets`), an optional vim layer, then the user's `keymap.json` next to
`settings.json`. Merged into a flat `Vec<KeyBinding>` and bound with `cx.bind_keys`.

## The file

```jsonc
[
  {
    "context": "Table && !Editing",          // optional key-context expression
    "use_key_equivalents": true,            // optional, for non-US layouts
    "bindings": {
      "ctrl-n": "table::SelectNext",                       // action
      "ctrl-k ctrl-s": ["view::Open", { "view": "logs" }], // action with data
      "ctrl-x": null                                        // unbind
    }
  }
]
```

Later sections and later bindings win; the user's file wins over vim, which wins over the
defaults. A `null` hides the key from the same or any lower layer. JSON with comments and
trailing commas is accepted (same parser as settings).

## Validation

Loading never fails as a whole. A file that is not valid JSON (or not a list) keeps the previous
keymap; a bad section, context, keystroke, binding value, action name or action data is skipped
and reported as a `KeymapDiagnostic` (logged, and available from `oxikube_keymap::diagnostics(cx)`
for a toast or the keymap editor, E21). Embedded layers skip bindings whose action no crate has
registered (the owning crate may not be in the build); the user's file reports them.

## Adding keys for a feature

1. Declare actions: `actions!(table, [SelectNext])`, or `#[derive(Action)]
   #[action(namespace = table)]` (+ `Deserialize`, `JsonSchema`) for actions with data.
   An action whose name is a declared `CommandId` *is* that command; the key, the palette and the
   MCP tool run the same behaviour (`ActionRegistry::command`).
2. Give views a key context with `KeyContextual` (`.key_context(self.key_context())`); standard
   names are in `oxikube_keymap::contexts`.
3. Add default bindings to `crates/platform/oxikube_assets/assets/keymaps/default-{macos,linux,windows}.json`
   (and `vim.json` when they make sense there).

### Off macOS, keep the terminal's keys

A focused terminal sends every plain `ctrl-` chord to its shell (`ctrl-w` deletes a word,
`ctrl-k` kills the line, `ctrl-q`, `ctrl-2..8`, `ctrl--`, `ctrl-alt-<letter>`, ...), and GPUI
matches bindings before the terminal sees the key. So an application shortcut in the `linux` /
`windows` defaults (or in the workspace's interim bindings) must be `ctrl-shift-<key>` (the
macOS `cmd` counterpart), or be unbound (`null`) in a `Terminal` section after it (not scoped `!Terminal`: GPUI
evaluates a negation false on an empty context stack, so the key would be dead while nothing is
focused). The `keymap_shadowing` test in `oxikube_terminal`
sweeps every key the terminal encodes (`mappings::to_esc_str`) against the shipped keymap and
fails on a shadowed one. The shipped defaults: `ctrl-shift-w` close tab, `ctrl-shift-b` / `-j` /
`-r` left / bottom / right dock, `ctrl-shift-k <arrow>` split, `ctrl-shift-q` quit,
`ctrl-shift-1..9` cluster tab, zoom `ctrl-=` / `ctrl--` / `ctrl-0` unbound in `Terminal`.

`ActionRegistry::from_app(cx)` lists names by namespace; `bindings_for_action_name(cx, name, data)`
lists an action's effective bindings (for the palette, E11).

## Wiring

`oxikube_keymap::init(cx)` after the settings store (it reads `<config dir>/keymap.json`, starts
the watcher). Bindings other crates added with `cx.bind_keys` survive reloads and rank below the
keymap layers; if such a crate initialises after the keymap, call `rebind(cx)` at the end of
start-up. The vim flag is `KeymapOptions::vim` / `set_vim_layer(cx, bool)`; the user-facing
`vim_mode` setting that drives it is wired once the settings schema generator links every
settings crate (E05-S06b, #454). Tests use `init_with_text` / `init_with_dir` (no watcher thread).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- platform crates (`oxikube_settings`, `oxikube_assets`)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

The design follows Zed's `keymap_file.rs`; the code is written from scratch (no Zed licence header).
