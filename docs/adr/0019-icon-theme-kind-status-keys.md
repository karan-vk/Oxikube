# ADR 0019: Icon themes use Zed's format, with kind and status keys in `file_icons`

- **Status:** Accepted (2026-11-03)
- **Deciders:** story E22-S11 (#293)
- **Related:** ADR 0004 (UI wrapper and Zed vendoring), ADR 0007 (extensions), ADR 0013 / 0016
  (performance budget), `crates/platform/oxikube_theme/src/icons/`

## Context

E22-S11 adds icon themes. Zed's icon-theme file format (`icon_themes/v0.2.0`) is the one Zed's
extensions ship and the one E23 extensions will ship, so Oxikube reads it unchanged. But it knows
only files and folders (`directory_icons`, `named_directory_icons`, `chevron_icons`,
`file_stems`, `file_suffixes`, `file_icons`), while Oxikube's icons are Kubernetes kinds and
statuses, plus the closed set of Lucide icons views name through `IconName`. There was no
kind-to-icon or status-to-icon map anywhere in the code base, only per-view `IconName::` uses.

## Decision

1. **Keys, not new fields.** A theme maps Oxikube's icons through ordinary `file_icons` keys with a
   prefix, so a Zed icon theme loads unchanged and an Oxikube one is still a valid Zed file:

   | Key | Overrides |
   |---|---|
   | `kind:<group>/<Kind>` | one kind of one API group (`kind:argoproj.io/Application`) |
   | `kind:<Kind>` | a kind of any group; core kinds have no group (`kind:Pod`) |
   | `status:<Status>` | a status text as Oxikube shows it (`status:CrashLoopBackOff`) |
   | `icon:<lucide-stem>` | one built-in icon everywhere it is drawn (`icon:box`) |

   Zed's `chevron_icons` and `directory_icons` map onto the chevron and folder icons; an explicit
   `icon:` key wins over them.
2. **Lucide is the default theme and the fallback.** The built-in theme is called `Lucide` and
   overrides nothing. A kind or status the selected theme has no key for falls back to its Lucide
   default (`oxikube_theme::icons::keys`: `default_kind_icon`, `default_status_icon`), through any
   `icon:` override of that icon. An unknown theme name shows Lucide.
3. **Views keep `IconName`.** The theme only changes the asset path an `IconName`, a kind or a
   status resolves to. `oxikube_ui::Icon::new` / `::kind` / `::status` ask the active resolver; no
   view names a path.
4. **Resolved once per theme change.** `IconResolver` turns the theme into a table indexed by
   `IconName` plus maps for kinds and statuses when the active icon theme changes. Drawing an icon
   is a lookup and a reference-count bump on the UI thread's copy of the resolver (a
   thread-local, because the component library's `From<Icon>` conversions have no `App`): no
   parsing, no allocation, no lock.
5. **Files are read off the UI thread and served from memory.** A theme's SVGs are read with the
   family file (watcher thread or background), every path checked to be relative, without `..`,
   an `.svg`, and (canonicalised) inside the icon theme's directory. They are mounted under
   `icon-themes/<id>/<path>` in `oxikube_assets::Assets`; the mount lives as long as a theme holds
   it, and a reload mounts under a new id so GPUI's atlas never shows a stale picture.
6. **Same registry, same sources.** Icon themes live in `ThemeRegistry` with the colour themes'
   `ThemeSource`s (`Bundled`, `User`, `Extension(id)`) and precedence, so E23-S06 loads and unloads
   an extension's icon themes with `load_icon_theme` / `remove_icon_themes` like its themes.

No Zed code is copied: the content structs are modelled on the documented file format, so no GPL
header or notices entry applies.

## Consequences

- Any Zed icon theme loads; in Oxikube it changes only chevrons and folders until it adds
  `kind:` / `status:` keys.
- A status key matches the text Oxikube shows, so it follows Oxikube's wording (`Terminating`,
  `CrashLoopBackOff`), not a separate enum. Adding a status to the defaults is one match arm.
- The thread-local copy assumes all drawing happens on the app's one foreground thread, which is
  how GPUI works; a test app that never installs icon themes on its thread draws Lucide.
