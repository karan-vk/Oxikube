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

## kdash (MIT) — https://github.com/kdash-rs/kdash
Copyright (c) 2021 Deepu K Sasidharan. Ported functions (tolerant kubeconfig loader, cronjob
trigger, merge-patch builders, log stream reconnect/dedup logic) keep the MIT notice in-file.

Entries:
- `crates/domain/oxikube_domain/src/age.rs` (test module `kdash_corpus`): test inputs and expected
  strings from `src/app/utils.rs` (`test_to_age`, `test_to_age_secs`), and the `duration_to_age`
  algorithm reimplemented as `AgeStyle::Detailed`. The MIT notice is reproduced next to the data:
  Copyright (c) 2021 Deepu K Sasidharan. Permission is hereby granted, free of charge, to any
  person obtaining a copy of this software and associated documentation files, to deal in the
  Software without restriction, subject to the inclusion of this copyright and permission notice
  in all copies or substantial portions of the Software. The Software is provided "as is",
  without warranty of any kind.

## deskribe (Apache-2.0) — https://github.com/nklmilojevic/deskribe
Used as a dependency. Its NOTICE (portions © The Kubernetes Authors, Apache-2.0) must be preserved
if any code is vendored.

## kubectl-view-allocations (CC0-1.0) — https://github.com/davidB/kubectl-view-allocations
Quantity parsing semantics referenced for `oxikube_domain::quantity` (no attribution required; noted for provenance).
`crates/domain/oxikube_domain/src/quantity.rs` follows the shape of `src/qty.rs` (`Qty` with a
scale table `Pi..n`, `FromStr`, `Display`, `Add`/`Sub`, percentage) and keeps an attribution
header. The value representation is rewritten as an exact `i128` nano-unit integer instead of
`f64`.

## Kubernetes apimachinery (Apache-2.0) — https://github.com/kubernetes/apimachinery
Semantics only, no code copied: quantity grammar and canonical formatting from
`pkg/api/resource/quantity.go`, and the `kubectl get` age cut-offs from `pkg/util/duration`
(`HumanDuration`), reimplemented in `oxikube_domain::quantity` and `oxikube_domain::age`. The test
vectors are written for Oxikube and are not copied from apimachinery's tests.

## gpui-kit / gpui-component (Apache-2.0) — https://github.com/longbridge/gpui-kit
Used as a dependency through `oxikube_ui`. Bundled Lucide icons (ISC) via `gpui-kit-assets`.

## Bundled themes
Theme families bundled under `crates/platform/oxikube_assets` list their own licence in the JSON `author`/`license` fields and here:
- (none yet)
