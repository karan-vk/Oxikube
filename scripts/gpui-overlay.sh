#!/usr/bin/env bash
# GPUI patch overlay (ADR 0017). Builds `.gpui-overlay/<crate>-<version>/` for every patch
# directory `patches/gpui/<crate>-<version>/`: the exact pinned crate from crates.io (sha256
# checked), with that directory's `NNNN-*.patch` files applied in order. The workspace
# `[patch.crates-io]` points the patched crates there.
#
#   scripts/gpui-overlay.sh          build what is missing or stale (a no-op when nothing changed)
#   scripts/gpui-overlay.sh --check  verify, without changing anything, that every overlay is
#                                    exactly its pinned crate plus its patches and that nothing
#                                    else is in `.gpui-overlay/`
#
# Needs only bash (3.2 works), tar, git, curl and shasum or sha256sum: it runs before cargo can
# resolve the workspace, which it cannot do while the overlay is missing.
#
# A patch directory holds:
#   checksum           the sha256 of `<crate>-<version>.crate`, as Cargo.lock recorded it before
#                      the crate was patched (and as the crates.io index lists it)
#   NNNN-<slug>.patch  unified diffs against the crate root (`a/src/...`), applied with `git apply`
set -euo pipefail

FORMAT=1
# GPUI_OVERLAY_ROOT is for the script's own tests (xtask check_gpui_pin::script_tests).
ROOT="${GPUI_OVERLAY_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
PATCHES="$ROOT/patches/gpui"
OVERLAY="$ROOT/.gpui-overlay"
STAMP=".oxikube-overlay-stamp"
CARGO_DIR="${CARGO_HOME:-$HOME/.cargo}"

mode=build
case "${1:-}" in
  "") ;;
  --check) mode=check ;;
  -h | --help)
    sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
    exit 0
    ;;
  *)
    echo "gpui-overlay: unknown argument \`$1\` (try --help)" >&2
    exit 2
    ;;
esac

die() {
  echo "gpui-overlay: error: $*" >&2
  exit 1
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

# `<crate>-<version>` -> crate and version (the version is the last `-` field: crate names have
# dashes, versions here do not).
split_name() {
  crate="${1%-*}"
  version="${1##*-}"
  case "$version" in
    [0-9]*.[0-9]*.[0-9]*) ;;
    *) die "patches/gpui/$1: the directory must be named <crate>-<version>, e.g. gpui-pre-macos-0.3.7" ;;
  esac
}

# The pinned version Cargo.lock resolves for $crate (a patched crate keeps its version but loses
# its registry source and checksum).
locked_versions() {
  awk -v want="$1" '
    /^\[\[package\]\]/ { name = ""; next }
    /^name = / { gsub(/"/, "", $3); name = $3; next }
    /^version = / && name == want { gsub(/"/, "", $3); print $3 }
  ' "$ROOT/Cargo.lock"
}

# The registry checksum Cargo.lock still records for $crate $version, if it is not patched yet.
locked_checksum() {
  awk -v want="$1" -v ver="$2" '
    /^\[\[package\]\]/ { name = ""; v = ""; next }
    /^name = / { gsub(/"/, "", $3); name = $3; next }
    /^version = / { gsub(/"/, "", $3); v = $3; next }
    /^checksum = / && name == want && v == ver { gsub(/"/, "", $3); print $3 }
  ' "$ROOT/Cargo.lock"
}

# The patch files of one patch directory, in apply order.
patch_files() {
  find "$1" -maxdepth 1 -type f -name '[0-9][0-9][0-9][0-9]-*.patch' | LC_ALL=C sort
}

# What a finished overlay records: the format, the crate, its checksum and every patch's hash.
expected_stamp() {
  local dir="$1" sum="$2" p
  echo "format $FORMAT"
  echo "crate $crate $version $sum"
  while IFS= read -r p; do
    [ -n "$p" ] && echo "patch $(sha256_of "$p") $(basename "$p")"
  done < <(patch_files "$dir")
}

# Copies the verified `.crate` to $1: from cargo's download cache, else from static.crates.io.
fetch_crate() {
  local out="$1" sum="$2" cached
  for cached in "$CARGO_DIR"/registry/cache/*/"$crate-$version.crate"; do
    if [ -f "$cached" ] && [ "$(sha256_of "$cached")" = "$sum" ]; then
      cp "$cached" "$out"
      return 0
    fi
  done
  local url="https://static.crates.io/crates/$crate/$crate-$version.crate"
  curl --fail --silent --show-error --location --retry 3 --output "$out" "$url" ||
    die "could not download $url (offline? a \`cargo fetch\` before the patch landed also fills $CARGO_DIR/registry/cache)"
  local got
  got="$(sha256_of "$out")"
  [ "$got" = "$sum" ] || die "$crate-$version.crate from $url has sha256 $got, but patches/gpui/$crate-$version/checksum pins $sum"
}

# Extracts the pinned crate into $1/$crate-$version and applies the patches of $2 in order.
assemble() {
  local into="$1" dir="$2" sum="$3" p
  fetch_crate "$into/$crate-$version.crate" "$sum"
  tar -xzf "$into/$crate-$version.crate" -C "$into"
  rm -f "$into/$crate-$version.crate"
  [ -f "$into/$crate-$version/Cargo.toml" ] || die "$crate-$version.crate has no $crate-$version/Cargo.toml"
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    # The ceiling keeps git from treating the Oxikube checkout around the overlay as the
    # repository: paths in the patch are relative to the crate root.
    if ! (cd "$into/$crate-$version" &&
      GIT_CEILING_DIRECTORIES="$into" git apply --whitespace=nowarn -p1 "$p"); then
      die "patches/gpui/$crate-$version/$(basename "$p") does not apply to the pinned $crate $version. \
Rebase it on the pinned crate (or delete it if upstream ships the fix) and run scripts/gpui-overlay.sh again."
    fi
  done < <(patch_files "$dir")
}

[ -d "$PATCHES" ] || die "no $PATCHES directory"
[ -f "$ROOT/Cargo.lock" ] || die "no Cargo.lock at $ROOT"

expected=""
problems=0
for dir in "$PATCHES"/*/; do
  [ -d "$dir" ] || continue
  dir="${dir%/}"
  name="$(basename "$dir")"
  split_name "$name"
  expected="$expected $name"

  [ -f "$dir/checksum" ] || die "patches/gpui/$name/checksum is missing (the sha256 of $name.crate)"
  sum="$(tr -d ' \n\r' <"$dir/checksum")"
  case "$sum" in
    *[!0-9a-f]* | "") die "patches/gpui/$name/checksum is not a sha256 hex digest" ;;
  esac
  [ ${#sum} -eq 64 ] || die "patches/gpui/$name/checksum is not a sha256 hex digest"
  [ -n "$(patch_files "$dir")" ] || die "patches/gpui/$name has no NNNN-*.patch file; delete the directory and its [patch.crates-io] entry instead"

  locked="$(locked_versions "$crate" | tr '\n' ' ')"
  case " $locked " in
    *" $version "*) ;;
    *) die "patches/gpui/$name: Cargo.lock pins $crate at ${locked:-nothing}, not $version. \
After a GPUI pin bump, rebase or delete the patches and rename the directory." ;;
  esac
  lock_sum="$(locked_checksum "$crate" "$version")"
  if [ -n "$lock_sum" ] && [ "$lock_sum" != "$sum" ]; then
    die "patches/gpui/$name/checksum ($sum) differs from Cargo.lock's checksum for $crate $version ($lock_sum)"
  fi

  want="$(expected_stamp "$dir" "$sum")"
  target="$OVERLAY/$name"

  if [ "$mode" = check ]; then
    if [ ! -f "$target/$STAMP" ]; then
      echo "gpui-overlay: $name is missing; run scripts/gpui-overlay.sh" >&2
      problems=$((problems + 1))
      continue
    fi
    if [ "$(cat "$target/$STAMP")" != "$want" ]; then
      echo "gpui-overlay: $name is stale (its crate or patches changed); run scripts/gpui-overlay.sh" >&2
      problems=$((problems + 1))
      continue
    fi
    scratch="$(mktemp -d "${TMPDIR:-/tmp}/gpui-overlay-check.XXXXXX")"
    assemble "$scratch" "$dir" "$sum"
    if ! diff -r -q -x "$STAMP" "$scratch/$name" "$target" >&2; then
      echo "gpui-overlay: $name differs from the pinned crate plus its patches (edited by hand?). \
Put the change in a patch under patches/gpui/$name, then rm -rf .gpui-overlay and run scripts/gpui-overlay.sh" >&2
      problems=$((problems + 1))
    fi
    rm -rf "$scratch"
    continue
  fi

  if [ -f "$target/$STAMP" ] && [ "$(cat "$target/$STAMP")" = "$want" ]; then
    continue
  fi
  mkdir -p "$OVERLAY"
  scratch="$(mktemp -d "$OVERLAY/.tmp.XXXXXX")"
  # A patch that does not apply must not leave the previous (now stale) overlay in place: cargo
  # would build it silently. Remove it first so the build fails until the patch is fixed.
  rm -rf "$target"
  trap 'rm -rf "$scratch"' EXIT
  assemble "$scratch" "$dir" "$sum"
  printf '%s\n' "$want" >"$scratch/$name/$STAMP"
  mv "$scratch/$name" "$target"
  rm -rf "$scratch"
  trap - EXIT
  echo "gpui-overlay: built .gpui-overlay/$name ($(patch_files "$dir" | wc -l | tr -d ' ') patch(es))"
done

# Anything else in the overlay is not from a pinned crate plus its patches.
if [ -d "$OVERLAY" ]; then
  for found in "$OVERLAY"/* "$OVERLAY"/.tmp.*; do
    [ -e "$found" ] || continue
    base="$(basename "$found")"
    case " $expected " in
      *" $base "*) continue ;;
    esac
    case "$base" in
      README.md) continue ;;
    esac
    if [ "$mode" = check ]; then
      echo "gpui-overlay: .gpui-overlay/$base has no patches/gpui/$base; run scripts/gpui-overlay.sh to remove it" >&2
      problems=$((problems + 1))
    else
      rm -rf "$found"
      echo "gpui-overlay: removed .gpui-overlay/$base (no patches/gpui/$base)"
    fi
  done
fi

if [ "$mode" = check ]; then
  [ "$problems" -eq 0 ] || die "$problems overlay problem(s)"
  echo "gpui-overlay: OK (${expected# })"
fi
