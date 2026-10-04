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
- (none yet)

### Zed's Apache-2.0 crates
Zed's GPUI crates (`gpui`, `gpui_tokio`, ...) are Apache-2.0, not GPL. Ported files keep the
Apache-2.0 notice (Copyright 2022 - 2025 Zed Industries, Inc.) and state their modifications.

Entries (file → upstream path @ rev):
- `crates/platform/oxikube_runtime/src/gpui_tokio.rs` → `crates/gpui_tokio/src/gpui_tokio.rs` @
  a84689073d296dfd39987bc7dd478e43ef76d83a (the `GlobalTokio` global, `init` / `init_from_handle`
  and the shutdown-on-drop; `Tokio::spawn`'s abort-on-drop guard is reworked as
  `oxikube_runtime::spawn_kube` in `kube_task.rs`). The `gpui_tokio` crate is not published in the
  `gpui-pre` snapshot family, so it is ported rather than depended on.

## kdash (MIT) — https://github.com/kdash-rs/kdash
Copyright (c) 2021 Deepu K Sasidharan. Ported functions (tolerant kubeconfig loader, cronjob
trigger, merge-patch builders, log stream reconnect/dedup logic) keep the MIT notice in-file.

Entries:
- `crates/adapters/oxikube_kube/src/kubeconfig/load.rs`: ports `is_blank_kubeconfig`,
  `load_kubeconfig_path`, `load_kubeconfig_from_paths` and `load_local_kubeconfig` from
  `src/network/mod.rs` @ c303673 (v2.1.1). Reworked to return per-file sources, context origins
  and diagnostics, and to take explicit inputs instead of reading the environment. The MIT
  notice and permission text are the file header (same text as below).
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

## Bundled themes
Theme families bundled under `crates/platform/oxikube_assets` list their own licence in the JSON `author`/`license` fields and here:
- (none yet)
