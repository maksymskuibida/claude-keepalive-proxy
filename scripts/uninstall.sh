#!/usr/bin/env bash
# Reverses scripts/install.sh: unloads the service first, then removes only the
# paths listed in the manifest (<home>/install-manifest), all inside the state folder.
#
# Usage: scripts/uninstall.sh [--home DIR] [--label NAME] [--dry-run]
#   --home DIR     state folder (default: $KEEPALIVE_PROXY_HOME, else ~/.keepalive-proxy; under
#                  sudo, the invoking user's ~/.keepalive-proxy, as install.sh does)
#   --label NAME   unload only the recorded service(s) with this label (default: every
#                  recorded service). A label with no recorded line is unloaded once, in the
#                  scope of the first recorded service.
#   --dry-run      print what would be done; change nothing
#
# Every recorded service is unloaded in its own scope. A system service needs root: when
# not root, the script names the remaining service and the sudo command, still unloads the
# others, keeps the manifest (reduced to what remains) and every file, and exits non-zero.
# Files and the manifest are removed only once all selected services are unloaded.
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

run_user=
if [ "$(id -u)" = 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
  case "$SUDO_USER" in *[!A-Za-z0-9._-]*) die "unusable SUDO_USER '$SUDO_USER'" ;; esac
  run_user=$SUDO_USER
fi
if [ -z "$home_dir" ]; then
  if [ -n "$run_user" ]; then
    user_home=$(eval "echo ~$run_user")
    case "$user_home" in /*) ;; *) die "cannot find the home of $run_user; pass --home DIR" ;; esac
    home_dir=$user_home/.keepalive-proxy
  else
    [ -n "${HOME:-}" ] || die "HOME is not set; pass --home DIR"
    home_dir=$HOME/.keepalive-proxy
  fi
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

# 1. The services first, so nothing keeps running from a binary that is about to go.
# Manifest line: service <system|user> <yes|no> [label].
lines=()
while IFS= read -r l; do
  case "$l" in "service "*) lines+=("$l") ;; esac
done < "$manifest"

# Select what to unload, as "<line>|<scope>|<label>"; <line> is empty for an extra unload.
plan=()
matched=0
for l in ${lines[@]+"${lines[@]}"}; do
  read -r _ scope given rec_label _ <<< "$l"
  svc_label=
  [ "$given" = yes ] && svc_label=$rec_label
  if [ -n "$label" ]; then
    [ "$svc_label" = "$label" ] || continue
    matched=1
  fi
  plan+=("$l|$scope|$svc_label")
done
if [ -n "$label" ] && [ "$matched" = 0 ]; then
  first_scope=user
  [ ${#lines[@]} -gt 0 ] && read -r _ first_scope _ <<< "${lines[0]}"
  plan+=("|$first_scope|$label")
fi

unloaded=() remaining=()
for item in ${plan[@]+"${plan[@]}"}; do
  l=${item%%|*}; rest=${item#*|}; scope=${rest%%|*}; svc_label=${rest#*|}
  args=(service uninstall)
  [ "$scope" = system ] && args+=(--system)
  [ -n "$svc_label" ] && args+=(--label "$svc_label")
  if [ "$dry" = 1 ]; then
    echo "would run: $bin ${args[*]}"
    continue
  fi
  if [ "$scope" = system ] && [ "$(id -u)" != 0 ]; then
    echo "uninstall: the system service${svc_label:+ '$svc_label'} needs root; run: sudo $0 --home $home_dir${svc_label:+ --label $svc_label}" >&2
    remaining+=("$l"); continue
  fi
  if [ ! -x "$bin" ]; then
    echo "uninstall: cannot unload the service: $bin is missing" >&2
    remaining+=("$l"); continue
  fi
  if [ -n "$run_user" ] && [ "$scope" = user ]; then
    sudo -u "$run_user" -H env KEEPALIVE_PROXY_HOME="$home_dir" "$bin" "${args[@]}" < /dev/null && ok=1 || ok=0
  else
    KEEPALIVE_PROXY_HOME=$home_dir "$bin" "${args[@]}" < /dev/null && ok=1 || ok=0
  fi
  if [ "$ok" = 1 ]; then unloaded+=("$l")
  else echo "uninstall: 'service uninstall${svc_label:+ --label $svc_label}' failed" >&2; remaining+=("$l"); fi
done

if [ ${#remaining[@]} -gt 0 ]; then
  # Keep the manifest and every file; drop only the service lines that are now unloaded.
  if [ ${#unloaded[@]} -gt 0 ]; then
    : > "$manifest.new"
    while IFS= read -r l; do
      drop=0
      for u in "${unloaded[@]}"; do [ "$l" = "$u" ] && drop=1; done
      [ "$drop" = 1 ] || printf '%s\n' "$l" >> "$manifest.new"
    done < "$manifest"
    mv -f "$manifest.new" "$manifest"
  fi
  echo "uninstall: not all services were unloaded; the manifest and all files are kept. Fix the above and run again." >&2
  exit 1
fi

# With --label, only the selected services were unloaded: the rest stay recorded, so
# files and manifest stay too.
if [ -n "$label" ] && [ "$dry" = 0 ]; then
  others=0
  for l in ${lines[@]+"${lines[@]}"}; do
    in_plan=0
    for u in ${unloaded[@]+"${unloaded[@]}"}; do [ "$l" = "$u" ] && in_plan=1; done
    [ "$in_plan" = 1 ] || others=1
  done
  if [ "$others" = 1 ]; then
    : > "$manifest.new"
    while IFS= read -r l; do
      drop=0
      for u in ${unloaded[@]+"${unloaded[@]}"}; do [ "$l" = "$u" ] && drop=1; done
      [ "$drop" = 1 ] || printf '%s\n' "$l" >> "$manifest.new"
    done < "$manifest"
    mv -f "$manifest.new" "$manifest"
    echo "uninstall: other recorded services remain loaded; the manifest and all files are kept." >&2
    exit 1
  fi
fi

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
