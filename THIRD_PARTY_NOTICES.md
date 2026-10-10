# Third-party notices

Oxikube is licensed under GPL-3.0-or-later (see `LICENSE`). It incorporates or is derived from the
following third-party work. Keep this file current: every vendored or ported module must add an
entry here and carry the original header in the file.

## Zed (GPL-3.0-or-later) — https://github.com/zed-industries/zed
Copyright © Zed Industries, Inc. Selected modules are vendored/ported (settings store and
comment-preserving JSON edits, keymap format and dispatch tests, theme-family schema loader,
picker delegate, terminal element, ACP thread model). Each file carries the header:

```
// Portions derived from Zed (https://github.com/zed-industries/zed), © Zed Industries, Inc.
// Licensed under GPL-3.0-or-later. Modifications © Oxikube contributors.
```

Entries (file → upstream path @ rev):
- `crates/platform/oxikube_settings/src/json_edit/mod.rs`, `json_edit/format.rs`:
  `update_value_in_json_text`, `replace_value_in_json_text`, `construct_json_value`,
  `infer_json_indent_size` and `to_pretty_json` from `crates/settings_json/src/settings_json.rs`
  @ a84689073d296dfd39987bc7dd478e43ef76d83a. Array-index (`#N`) key paths not vendored; adapted
  to tree-sitter 0.26 (the version gpui-component's editor links; `QueryMatch::captures` is a field).
- `crates/platform/oxikube_settings/src/json_edit/tests.rs`: the `object_replace`,
  `object_replace_escapes_new_key`, `object_remove_and_rename_find_an_escaped_key_by_its_own_range`
  and `test_infer_json_indent_size` tests from the same file @ a84689073d.
- `crates/platform/oxikube_settings/src/settings.rs`, `store/mod.rs`, `store/value.rs`,
  `update/mod.rs`: the `Settings` trait shape, the type-erased `SettingValue`/`AnySettingValue`
  slots, inventory registration and `edits_for_update`, derived from
  `crates/settings/src/settings_store.rs` @ a84689073d (rewritten for per-crate content types,
  a cluster layer and change-tracking generations).

### Zed's Apache-2.0 crates
Zed's GPUI crates (`gpui`, `gpui_tokio`, ...) are Apache-2.0, not GPL. Ported files keep the
Apache-2.0 notice (Copyright 2022 - 2025 Zed Industries, Inc.) and state their modifications.

Entries (file → upstream path @ rev):
- `crates/platform/oxikube_runtime/src/gpui_tokio.rs` → `crates/gpui_tokio/src/gpui_tokio.rs` @
  a84689073d296dfd39987bc7dd478e43ef76d83a (the `GlobalTokio` global, `init` / `init_from_handle`
  and the shutdown-on-drop; `Tokio::spawn`'s abort-on-drop guard is reworked as
  `oxikube_runtime::spawn_kube` in `kube_task.rs`). The `gpui_tokio` crate is not published in the
  `gpui-pre` snapshot family, so it is ported rather than depended on.

### GPUI patch overlay (Apache-2.0 crates, GPL-3.0-or-later patches)
`patches/gpui/` holds unified diffs against the exact pinned `gpui-pre-macos` and `gpui-pre-apple`
0.3.7 crates from crates.io (snapshots of Zed's `crates/gpui_macos` and `crates/gpui_apple` @
1a28cff, Apache-2.0, Copyright Zed Industries, Inc. and contributors). `scripts/gpui-overlay.sh`
applies them to the downloaded crates in the gitignored `.gpui-overlay/` (ADR 0017); the crates'
own `LICENSE-APACHE` files stay in place there. The patches are Copyright Oxikube contributors,
GPL-3.0-or-later (each patch header says so, names the upstream draft, and states the change);
the patched crates as built into Oxikube are distributed under GPL-3.0-or-later with the
Apache-2.0 notices of the originals. Entries (patch → upstream path @ rev):
- `patches/gpui/gpui-pre-macos-0.3.7/0001-overlay-allow-warnings.patch`,
  `0002-draw-late-resize-in-its-transaction.patch` → `crates/gpui_macos/src/{gpui_macos.rs,
  window.rs, display_link.rs}` @ 1a28cff.
- `patches/gpui/gpui-pre-apple-0.3.7/0001-overlay-allow-warnings.patch`,
  `0002-prefetch-next-drawable.patch` → `crates/gpui_apple/src/{gpui_apple.rs,
  metal_renderer.rs}` @ 1a28cff, plus the new `drawable_prefetch.rs`.

## deskribe (Apache-2.0) — https://github.com/nklmilojevic/deskribe
Copyright 2026 Nikola Milojevic. The native `kubectl describe` renderer behind
`oxikube_describe::NativeDescribe` (E07-S06). It is used as a crate dependency (`deskribe` in
`[workspace.dependencies]`), not vendored: no deskribe source is copied into this repository, so
there is no in-file header to carry. Its own sources keep their SPDX headers and its `NOTICE`
applies to what it ships: it is an adaptation of Kubernetes code (kubernetes/kubectl
`pkg/describe` v0.35.1 with v0.37.0 behaviour, kubernetes/apimachinery quantity and duration
formatting, kubernetes/component-helpers resource accounting), Copyright 2014 and other years
as noted in its source files, The Kubernetes Authors, licensed under the Apache License 2.0.
If any deskribe code is ever vendored, the file must keep its Apache-2.0 header and this entry
must name it.

## kdash (MIT) — https://github.com/kdash-rs/kdash
Copyright (c) 2021 Deepu K Sasidharan. Ported functions (tolerant kubeconfig loader, cronjob
trigger, merge-patch builders, log stream reconnect/dedup logic) keep the MIT notice in-file.

Entries:
- `crates/adapters/oxikube_kube/src/kubeconfig/load.rs`: ports `is_blank_kubeconfig`,
  `load_kubeconfig_path`, `load_kubeconfig_from_paths` and `load_local_kubeconfig` from
  `src/network/mod.rs` @ c303673 (v2.1.1). Reworked to return per-file sources, context origins
  and diagnostics, and to take explicit inputs instead of reading the environment. The MIT
  notice and permission text are the file header (same text as below).
- `crates/adapters/oxikube_kube/src/logs/follow/mod.rs`: ports `stream_container_logs` (reconnect
  loop with a `since` overlap, backoff, dedup against recent lines, size/time batching) and
  `fetch_previous_logs` from `src/network/stream.rs` @ c303673 (v2.1.1). Reworked to emit `LogLine`s
  over a bounded channel, resume from the last kubelet timestamp, dedup on (timestamp, text), read
  a restarted container's previous instance and end when the pod is gone. The MIT notice and
  permission text are the file header (same text as below). `logs/follow/resume.rs` and
  `logs/dedup.rs` carry the derived reconnect decisions of the same port.
- `crates/adapters/oxikube_kube/src/subresource/patches.rs`: ports `ResourcePatch::to_merge_patch`
  (rollout-restart annotation, cordon and uncordon, cronjob suspend, scale replicas) from
  `src/network/mod.rs` @ c303673 (v2.1.1). Reworked so the restart timestamp is an argument and
  the result is an `oxikube_ports::Patch`; pinned by exact-JSON tests. The MIT notice and
  permission text are the file header (same text as below).
- `crates/adapters/oxikube_kube/src/algorithms/cronjob.rs`: ports `trigger_cronjob` (clone
  `spec.jobTemplate` into a `Job` with `generateName: <cronjob>-manual-`, the
  `cronjob.kubernetes.io/instantiate` annotation and an owner reference to the CronJob) from
  `src/network/mod.rs` @ c303673 (v2.1.1). Reworked to run on a `ResourcePort`, return the created
  Job, take write options (dry run), cut the generated-name prefix to the server's limit and
  report a CronJob with no template or uid as a validation error. The MIT notice and permission
  text are the file header (same text as below).
- `crates/domain/oxikube_domain/src/age.rs` (test module `kdash_corpus`): test inputs and expected
  strings from `src/app/utils.rs` (`test_to_age`, `test_to_age_secs`), and the `duration_to_age`
  algorithm reimplemented as `AgeStyle::Detailed`. The full MIT licence text
  is in the `age.rs` file header and reproduced here:

```text
Copyright (c) 2021 Deepu K Sasidharan

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Lucide icons (ISC) — https://lucide.dev
Copyright (c) 2026 Lucide Icons and Contributors; some icons derive from Feather (MIT). The SVGs
under `crates/platform/oxikube_assets/assets/icons/` are copied unmodified from the Lucide set
bundled in `gpui-kit-assets` 0.7.0 (the curated subset listed in `src/icons.rs`). The full licence
text, including the Feather attribution, is `crates/platform/oxikube_assets/assets/icons/LICENSE`.

## gpui-component (Apache-2.0) — https://github.com/longbridge/gpui-kit
Used as a dependency of `oxikube_ui` only; no source is vendored. `oxikube_ui` reuses its bundled
default icon set at runtime through `gpui-kit-assets`.

## deskribe (Apache-2.0) — https://github.com/nklmilojevic/deskribe
Used as a dependency. Its NOTICE (portions © The Kubernetes Authors, Apache-2.0) must be preserved
if any code is vendored.

## kubectl-view-allocations (CC0-1.0) — https://github.com/davidB/kubectl-view-allocations
Quantity parsing semantics referenced for `oxikube_domain::quantity` (no attribution required; noted for provenance).
`crates/domain/oxikube_domain/src/quantity.rs` follows the shape of `src/qty.rs` (`Qty` with a
scale table `Pi..n`, `FromStr`, `Display`, `Add`/`Sub`, percentage) and keeps an attribution
header. The value representation is rewritten as an exact decimal (`i128` mantissa, `i32`
exponent) instead of `f64`.

## Kubernetes apimachinery (Apache-2.0) — https://github.com/kubernetes/apimachinery
Semantics only, no code copied: quantity grammar and canonical formatting from
`pkg/api/resource/quantity.go`, and the `kubectl get` age cut-offs from `pkg/util/duration`
(`HumanDuration`), reimplemented in `oxikube_domain::quantity` and `oxikube_domain::age`.

Test data ported (Copyright 2014 The Kubernetes Authors, Apache-2.0):
- `crates/domain/oxikube_domain/tests/quantity_corpus/apimachinery.rs`: the input tables of
  `pkg/api/resource/quantity_test.go` @ v0.37.0 (`TestQuantityParse` with its `-`/`+` loops and
  invalid list, `TestQuantityParseEmit`, `TestQuantityString`, `TestParseQuantityString`,
  `TestParseQuantity`), with expected values, formats and `String()` text generated once by
  running apimachinery v0.37.0. The file carries the Apache-2.0 header; the licence text is at
  http://www.apache.org/licenses/LICENSE-2.0.

## Kubernetes printers (Apache-2.0) — https://github.com/kubernetes/kubernetes
Semantics only, no code copied: the `kubectl get` column rules of `printPod`, `printNode`
(`findNodeRoles`), `printJob`, `printCronJob` and the apps workload printers in
`pkg/printers/internalversion/printers.go`, reimplemented over raw JSON in
`oxikube_domain::view`. The test cases are written for Oxikube.

## gpui-kit / gpui-component (Apache-2.0) — https://github.com/longbridge/gpui-kit
Used as a dependency through `oxikube_ui`. Bundled Lucide icons (ISC) via `gpui-kit-assets`.

## alacritty_terminal (Apache-2.0) — https://github.com/alacritty/alacritty
Used as a dependency of `oxikube_terminal` only (pinned `=0.26.0`, with its `vte` parser); no
source is vendored. `oxikube_terminal::grid` wraps its `Term` behind Oxikube types, written from
the crate's documentation (Zed's GPL `terminal` crate was not copied).

## Bundled themes
Theme families bundled under `crates/platform/oxikube_assets` list their own licence in the JSON `author`/`license` fields and here:
- One Dark and One Light (MIT, Copyright (c) 2014 GitHub Inc., from Atom's `one-dark-ui` /
  `one-light-ui`; the theme file is Zed's `assets/themes/one/one.json` @
  56cf49bc1afe05bbc777a7df5a01f79299ab4956): `crates/platform/oxikube_assets/assets/themes/one.json`,
  licence text in `assets/themes/LICENSES.md` next to it. Bundled, and the fallback for any key
  a user theme leaves unset. Oxikube modifies the file in one respect: the text and status colours
  (`text.muted`, `text.accent`, `error`, `info`, `success`, `warning`) are adjusted in lightness so
  they reach WCAG AA (4.5:1) on the backgrounds they are drawn on (E05-U562; checked by
  `oxikube_ui::tokens::contrast`).

Test-only fixtures (not bundled, not shipped in the application): `crates/platform/oxikube_theme/tests/fixtures/`
holds Zed's `ayu/ayu.json` (MIT, Copyright (c) 2016 Ike Ku, https://github.com/dempfi/ayu) and
`gruvbox/gruvbox.json` (MIT, https://github.com/morhetz/gruvbox), copied from Zed's `assets/themes`
@ 56cf49bc1afe05bbc777a7df5a01f79299ab4956 with their licences in `tests/fixtures/LICENSES.md`.
They are the import tests' input (E05-S08). Do not bundle another family without adding it here
and checking its licence in Zed's `assets/themes/LICENSES`.

`oxikube_theme` reads Zed's theme-family *file format* (schema v0.2.0) but contains no Zed code:
its types and the key mapping table are our own.
