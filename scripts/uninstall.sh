#!/usr/bin/env bash
# Reverses scripts/install.sh: unloads the service first, then removes only the
# paths listed in the manifest (<home>/install-manifest), all inside the state folder.
#
# Usage: scripts/uninstall.sh [--home DIR] [--label NAME] [--dry-run]
#   --home DIR     state folder (default: $KEEPALIVE_PROXY_HOME, else ~/.keepalive-proxy)
#   --label NAME   service label to unload (default: the one recorded in the manifest)
#   --dry-run      print what would be done; change nothing
#
# Files in the state folder that are not in the manifest (settings, logs, anything
# else) are left alone, and so is a folder that is not empty afterwards.
set -eu

usage="usage: uninstall.sh [--home DIR] [--label NAME] [--dry-run]"
die() { echo "uninstall: $*" >&2; exit 1; }

home_dir=${KEEPALIVE_PROXY_HOME:-}
label=${KEEPALIVE_PROXY_LABEL:-}
dry=0
while [ $# -gt 0 ]; do
  case "$1" in
    --home|--label)
      [ $# -ge 2 ] || die "$1 needs a value ($usage)"
      if [ "$1" = --home ]; then home_dir=$2; else label=$2; fi
      shift 2 ;;
    --dry-run) dry=1; shift ;;
    -h|--help) echo "$usage"; exit 0 ;;
    *) die "unknown argument '$1' ($usage)" ;;
  esac
done

if [ -z "$home_dir" ]; then
  [ -n "${HOME:-}" ] || die "HOME is not set; pass --home DIR"
  home_dir=$HOME/.keepalive-proxy
fi
case "$home_dir" in /*) ;; *) die "the state folder must be an absolute path: $home_dir" ;; esac
if [ -n "$label" ]; then
  case "$label" in *[!A-Za-z0-9._-]*) die "invalid label '$label'" ;; esac
fi

manifest=$home_dir/install-manifest
bin=$home_dir/bin/keepalive-proxy
[ -f "$manifest" ] || die "no manifest at $manifest; nothing to remove"

# A manifest path must be a plain relative path inside the state folder.
safe_rel() {
  case "$1" in
    ""|/*|..|../*|*/..|*/../*) return 1 ;;
  esac
  return 0
}

# 1. The service first, so nothing keeps running from a binary that is about to go.
while IFS=' ' read -r kind scope given rec_label _; do
  [ "$kind" = service ] || continue
  # Manifest line: service <system|user> <yes|no> [label]. A --label argument wins.
  svc_label=
  if [ -n "$label" ]; then svc_label=$label
  elif [ "$given" = yes ]; then svc_label=$rec_label; fi
  args=(service uninstall)
  [ "$scope" = system ] && args+=(--system)
  [ -n "$svc_label" ] && args+=(--label "$svc_label")
  if [ "$dry" = 1 ]; then
    echo "would run: $bin ${args[*]}"
    continue
  fi
  [ -x "$bin" ] || die "cannot unload the service: $bin is missing. Nothing was removed."
  KEEPALIVE_PROXY_HOME=$home_dir "$bin" "${args[@]}" \
    || die "'service uninstall' failed; nothing was removed"
done < "$manifest"

# 2. Files listed in the manifest.
while IFS=' ' read -r kind rel _; do
  [ "$kind" = file ] || continue
  rel_path=$rel
  safe_rel "$rel_path" || die "unsafe path in manifest: $rel_path"
  target=$home_dir/$rel_path
  [ "$rel_path" = install-manifest ] && continue   # removed last
  if [ "$dry" = 1 ]; then echo "would remove $target"; continue; fi
  if [ -f "$target" ] || [ -L "$target" ]; then rm -f "$target"; echo "removed $target"; fi
done < "$manifest"

# 3. Folders we created, deepest first, only when empty.
dirs=$(grep -E '^dir ' "$manifest" | sed 's/^dir //' | awk '{ print length, $0 }' | sort -rn | cut -d' ' -f2- || true)
if [ "$dry" = 1 ]; then
  echo "would remove $manifest"
  [ -n "$dirs" ] && echo "$dirs" | while IFS= read -r d; do echo "would remove folder $home_dir/$d if empty"; done
  exit 0
fi
rm -f "$manifest"; echo "removed $manifest"
if [ -n "$dirs" ]; then
  while IFS= read -r d; do
    if [ "$d" = . ]; then target=$home_dir; else
      safe_rel "$d" || die "unsafe path in manifest: $d"
      target=$home_dir/$d
    fi
    if [ -d "$target" ] && rmdir "$target" 2>/dev/null; then echo "removed folder $target"; fi
  done <<EOF
$dirs
EOF
fi
echo "uninstall: done"
