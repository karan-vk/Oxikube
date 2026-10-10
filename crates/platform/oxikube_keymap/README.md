# oxikube_keymap

**Layer:** `platform`

Layered, hot-reloading key bindings in Zed's `keymap.json` format (E05-S07): per-OS defaults
(embedded in `oxikube_assets`), an optional vim layer, then the user's `keymap.json` next to
`settings.json`. Merged into a flat `Vec<KeyBinding>` and bound with `cx.bind_keys`.

## The file

```jsonc
[
  {
    "context": "ResourceTable && !Editing",   // optional key-context expression
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

### Contexts and the k9s verbs (E11-S07)

Each view sets its own context with `KeyContextual`; the deeper context wins. The Phase 1 names
(`contexts::PHASE_1`) and what sets them:

| Context | Set by | Also carries |
|---|---|---|
| `Workspace` | the workspace root (and each cluster tab's own workspace) | |
| `ClusterTab` | a cluster tab, above everything in it | `connected` |
| `ResourceTable` | a resource table | `Editing`, `selection`, `kind` (`Pod`), `scope` |
| `DetailDrawer` | the resource detail (drawer or pinned tab) | `mount`, `kind` |
| `LogView` | the log viewer | `Editing`, `searching` |
| `Terminal` | a terminal | `searching` |
| `ManifestEditor` | the manifest editor (E10) | |
| `Palette`, `JumpBar` | the command palette and `:` bar (E11-S03, S05) | |
| `Help` | the help overlay (E11-S10) | `empty` while its search field is empty |

Bare-letter verbs are bound only where a letter is not text, and only while no field has the focus
(`ResourceTable && !Editing`): `y` YAML, `d` describe, `e` edit, `ctrl-d` delete, `l` logs, `s`
shell, `shift-f` forward a port, `f` the forwards, `ctrl-w` wide columns, `/` filter. `:` (jump bar)
and `?` (help) are bound in `ClusterTab` and set to `null` in `Terminal || ManifestEditor || Palette
|| JumpBar || Input || Editing`, so they are characters wherever text is typed. A key bound to an
action nobody registered yet (the editor, port forwarding, the jump bar, the help overlay) is
skipped in the embedded layers and starts working when its crate declares the action.

Scope a binding to a kind with the context values: `{ "context": "ResourceTable && !Editing && kind
== Deployment", "bindings": { "s": "..." } }`. `ctrl-w` is wide columns in a table and
close-the-tab (`cmd-w` on macOS, `ctrl-shift-w` elsewhere) never collide: the table section is
`ResourceTable`, the tab key is the workspace's.

A view's action *stands for* a command (`resource_table::ViewYaml` is `resource::ViewYaml`); the
pairing is `stands_for::STANDS_FOR`, which `bindings_for_command(cx, id)` reads to find the key of a
command and `tests/defaults.rs` reads to fail on a default that dispatches nothing.
`dispatch::resolve(cx, "y", &parse_stack(&["Workspace", "ResourceTable kind=Pod"])?)` says what a key
does in a context stack written as data, and `active_bindings` lists what is in force there.

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

## The vim base keymap (E11-S09)

`"base_keymap": "vim"` in `settings.json` (`"default"` is the default; schema in
`settings.schema.json`, default in `default.json`) layers `vim.json` between the per-OS defaults
and the user's `keymap.json`. `init*` reads the setting before the first merge and follows it, so
editing `settings.json` swaps the layer and rebinds without a restart (a reload that leaves the
value equal rebinds nothing). The setting is `oxikube_keymap::KeymapSettings` (module
`base_keymap`).

The layer is one section, `ResourceTable && !Editing`, so it never reaches a text field, the
terminal, the manifest editor, the palette or the jump bar (`tests/vim.rs` resolves every vim key
in those contexts with and without the layer and requires the same answer):

| Key | Action |
|---|---|
| `j` / `k`, `g g` / `shift-g` | next / previous row, first / last row |
| `ctrl-d` / `ctrl-u`, `ctrl-f` / `ctrl-b` | half a page / a page down and up |
| `/` | focus the table's filter (the E07-S04 bar) |
| `:` | the jump bar: the defaults' `ClusterTab` binding, nothing vim-specific |
| `d d` | `resource_table::DeleteSelected`: the delete dialog, so read-only, the confirmation tier and the audit record apply |
| `y y` | `resource_table::CopyName` (`resource::CopyName`) |
| `g d` / `g y` | describe / YAML, which move off `d` / `y` |

Overrides of the defaults, stated in the file: `ctrl-d` (k9s delete) becomes half a page, and the
single `d` / `y` are `null` so they stay pending for their second key instead of firing the k9s
verb first. GPUI replays a pair that does not complete as two fresh keys: `d` then `j` is `j`,
never a delete. The detail drawer keeps `d` / `y` / `j` / `k`. The k9s base keymap (E21-S09) is
the next value of the same setting.

## The user's keymap.json (E11-S08)

`<config dir>/keymap.json` (`$OXIKUBE_CONFIG_DIR` or the OS default) layers over the defaults and
the optional vim layer. `keymap::OpenUser` ("Open User Keymap" in the palette) creates it from a
commented template and opens it; an existing file is never overwritten.

- **Failure policy**: a bad binding is skipped and the others apply; a file that is not JSON keeps
  the previous keymap. `null` on a key nobody bound is fine.
- **Notification**: every problem has a line (`KeymapDiagnostic::line`, displayed
  `keymap.json:12: unknown action `x::Y` (binding `cmd-k`)`). `subscribe_diagnostics` raises a
  `KeymapDiagnosticsEvent` when the list changes (empty when fixed) and
  `KeymapDiagnosticsEvent::message` is the text of the one summarising toast; the binary shows it
  (the keymap is platform code and cannot).
- **Hot reload**: the watcher (the settings crate's, on the parent directory, debounced) reads and
  parses the file on its own thread (`ParsedUserKeymap`); the UI thread checks the actions and
  replaces the keymap's layers in one call. Tests use `init_with_dir` (no watcher) and `reload(cx)`
  or `reload_user_keymap(cx, text)`.
- **Sources and conflicts**: each resolved binding keeps its layer (`KeybindSource`: default, vim
  base or user) in `BindingInfo::layer`; `conflicts(cx)` lists keys bound twice, to different
  things, in one context of one layer (the later wins).

## Wiring

`oxikube_keymap::init(cx)` after the settings store (it reads `<config dir>/keymap.json`, starts
the watcher). Bindings other crates added with `cx.bind_keys` survive reloads and rank below the
keymap layers; if such a crate initialises after the keymap, call `rebind(cx)` at the end of
start-up. The vim layer is the `base_keymap` setting (below); `set_vim_layer(cx, bool)` and
`KeymapOptions::vim` are what it drives (and what tests use without a settings store). Tests use `init_with_text` / `init_with_dir` (no watcher thread).

## Allowed internal dependencies

- `oxikube_domain`
- `oxikube_ports`
- platform crates (`oxikube_settings`, `oxikube_assets`)

See `docs/ARCHITECTURE.md` for the full dependency rules. `cargo xtask lint-deps` fails CI when this crate depends on anything outside its layer rules.

The design follows Zed's `keymap_file.rs`; the code is written from scratch (no Zed licence header).
