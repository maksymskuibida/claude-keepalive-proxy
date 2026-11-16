#!/usr/bin/env bash
# Tests for scripts/install.sh and scripts/uninstall.sh in a temp folder, always with
# --no-service (or --dry-run) so launchd is never touched.
set -u
src=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

# Never touch the real state folder or service settings.
unset KEEPALIVE_PROXY_HOME KEEPALIVE_PROXY_LABEL KEEPALIVE_PROXY_PORT
state=$work/state
real_home=${HOME:-}
export HOME=$work/fakehome; mkdir -p "$HOME"

# Dry run changes nothing and does not build.
out=$(bash "$src/scripts/install.sh" --home "$state" --dry-run) && [ ! -e "$state" ] \
  && ok "install --dry-run changes nothing" || { bad "install --dry-run changes nothing"; echo "$out"; }

# Real install (cargo build) with --no-service.
out=$(bash "$src/scripts/install.sh" --home "$state" --no-service 2>&1) \
  && ok "install --no-service succeeds" || { bad "install --no-service succeeds"; echo "$out"; }
[ -x "$state/bin/keepalive-proxy" ] && ok "binary installed" || bad "binary installed"
[ -f "$state/install-manifest" ] && ok "manifest written" || bad "manifest written"
grep -qxF "file bin/keepalive-proxy" "$state/install-manifest" && ok "manifest lists the binary" || bad "manifest lists the binary"
! grep -q '^service ' "$state/install-manifest" && ok "no service recorded with --no-service" || bad "no service recorded with --no-service"

# The installed binary runs.
"$state/bin/keepalive-proxy" service install --dry-run --label local.testonly >/dev/null 2>&1 \
  && ok "installed binary runs" || bad "installed binary runs"

# A file that is not in the manifest survives uninstall.
echo keep > "$state/precious.txt"
out=$(bash "$src/scripts/uninstall.sh" --home "$state" --dry-run) && [ -x "$state/bin/keepalive-proxy" ] \
  && ok "uninstall --dry-run removes nothing" || { bad "uninstall --dry-run removes nothing"; echo "$out"; }
out=$(bash "$src/scripts/uninstall.sh" --home "$state" 2>&1) \
  && ok "uninstall succeeds" || { bad "uninstall succeeds"; echo "$out"; }
[ ! -e "$state/bin" ] && [ ! -e "$state/install-manifest" ] && ok "installed files removed" || bad "installed files removed"
[ "$(cat "$state/precious.txt" 2>/dev/null)" = keep ] && ok "file outside the manifest survives" || bad "file outside the manifest survives"
rm -rf "$state"

# Fully removed: when the install created the folder, nothing is left.
bash "$src/scripts/install.sh" --home "$state" --no-service >/dev/null 2>&1
bash "$src/scripts/uninstall.sh" --home "$state" >/dev/null 2>&1
[ ! -e "$state" ] && ok "install tree fully removed" || { bad "install tree fully removed"; ls -R "$state"; }

# A folder from the environment works too.
KEEPALIVE_PROXY_HOME=$work/envstate bash "$src/scripts/install.sh" --no-service --bin "$work/nonexistent" >/dev/null 2>&1 \
  && bad "missing --bin is an error" || ok "missing --bin is an error"

# Unsafe manifest paths are refused and nothing outside is deleted.
mkdir -p "$work/evil"; echo x > "$work/victim"
printf 'file ../victim\n' > "$work/evil/install-manifest"
bash "$src/scripts/uninstall.sh" --home "$work/evil" >/dev/null 2>&1 \
  && bad "unsafe manifest path refused" || ok "unsafe manifest path refused"
[ -f "$work/victim" ] && ok "file outside the state folder untouched" || bad "file outside the state folder untouched"

# A real label named "default" is unloaded by name; an unset label passes no --label.
stub=$work/stub-proxy
printf '#!/bin/sh\necho "$@" >> "%s"\n' "$work/stub.log" > "$stub"; chmod 755 "$stub"
for lbl in default - ""; do
  : > "$work/stub.log"; st=$work/stub-state
  if [ -n "$lbl" ]; then la=(--label "$lbl"); else la=(); fi
  bash "$src/scripts/install.sh" --home "$st" --bin "$stub" ${la[@]+"${la[@]}"} >/dev/null 2>&1
  bash "$src/scripts/uninstall.sh" --home "$st" >/dev/null 2>&1
  want_i="service install"; want_u="service uninstall"
  if [ -n "$lbl" ]; then want_i="$want_i --label $lbl"; want_u="$want_u --label $lbl"; fi
  [ "$(cat "$work/stub.log")" = "$want_i
$want_u" ] && ok "service label '${lbl:-unset}' recorded and unloaded the same" \
    || { bad "service label '${lbl:-unset}' recorded and unloaded the same"; cat "$work/stub.log"; }
  rm -rf "$st"
done

# Relabelled reinstall: both services are unloaded by one uninstall.
out=$(bash "$src/tests/scripts/probe_relabel.sh" 2>&1) && ok "relabel reinstall unloads both services" \
  || { bad "relabel reinstall unloads both services"; echo "$out"; }

# An unrecorded binary stops the install and leaves no folders behind.
mkdir -p "$work/foreign-parent"
fst=$work/foreign-parent/state
mkdir -p "$fst/bin"; echo mine > "$fst/bin/keepalive-proxy"
bash "$src/scripts/install.sh" --home "$fst" --bin "$stub" --no-service >/dev/null 2>&1 \
  && bad "foreign binary refused" || ok "foreign binary refused"
[ "$(cat "$fst/bin/keepalive-proxy")" = mine ] && [ ! -e "$fst/install-manifest" ] \
  && ok "foreign binary untouched" || bad "foreign binary untouched"
rm -rf "$fst"

# Scope change on reinstall: the user service is unloaded, the system one (needs root) is
# named with the sudo command, nothing else is removed, the exit code is non-zero.
if [ "$(id -u)" != 0 ]; then
  : > "$work/stub.log"; st=$work/scope-state
  bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.a >/dev/null 2>&1
  bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.b --system >/dev/null 2>&1
  : > "$work/stub.log"
  err=$(bash "$src/scripts/uninstall.sh" --home "$st" 2>&1 >/dev/null) && bad "mixed scopes: uninstall exits non-zero" \
    || ok "mixed scopes: uninstall exits non-zero"
  grep -qx "service uninstall --label local.a" "$work/stub.log" && ok "mixed scopes: user service unloaded" \
    || bad "mixed scopes: user service unloaded"
  ! grep -q -- "--system" "$work/stub.log" && ok "mixed scopes: system service not run without root" \
    || bad "mixed scopes: system service not run without root"
  case "$err" in *"sudo "*local.b*) ok "mixed scopes: sudo command named" ;; *) bad "mixed scopes: sudo command named"; echo "$err" ;; esac
  [ -x "$st/bin/keepalive-proxy" ] && grep -qx "service system yes local.b" "$st/install-manifest" \
    && ! grep -q "service user yes local.a" "$st/install-manifest" \
    && ok "mixed scopes: files kept, manifest reduced to the remaining service" \
    || bad "mixed scopes: files kept, manifest reduced to the remaining service"
  rm -rf "$st"
fi

# --label selects only the recorded lines with that label; the others stay recorded.
: > "$work/stub.log"; st=$work/label-state
bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.a >/dev/null 2>&1
bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.b >/dev/null 2>&1
: > "$work/stub.log"
bash "$src/scripts/uninstall.sh" --home "$st" --label local.a >/dev/null 2>&1 && bad "--label with other services left: non-zero" \
  || ok "--label with other services left: non-zero"
[ "$(cat "$work/stub.log")" = "service uninstall --label local.a" ] && ok "--label unloads only the matching line" \
  || { bad "--label unloads only the matching line"; cat "$work/stub.log"; }
grep -qx "service user yes local.b" "$st/install-manifest" && [ -x "$st/bin/keepalive-proxy" ] \
  && ok "--label keeps the other service recorded and the files" || bad "--label keeps the other service recorded and the files"
: > "$work/stub.log"
bash "$src/scripts/uninstall.sh" --home "$st" >/dev/null 2>&1 && [ ! -e "$st" ] \
  && [ "$(cat "$work/stub.log")" = "service uninstall --label local.b" ] \
  && ok "a later uninstall unloads the rest and removes everything" || bad "a later uninstall unloads the rest and removes everything"
# A label with no recorded line is unloaded once, in the first recorded scope.
: > "$work/stub.log"
bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.a >/dev/null 2>&1
: > "$work/stub.log"
bash "$src/scripts/uninstall.sh" --home "$st" --label local.x >/dev/null 2>&1
[ "$(cat "$work/stub.log")" = "service uninstall --label local.x" ] && ok "unrecorded --label unloaded once" \
  || { bad "unrecorded --label unloaded once"; cat "$work/stub.log"; }
rm -rf "$st"

# Not root: SUDO_USER is ignored and HOME is used for the default state folder.
bash "$src/scripts/install.sh" --bin "$stub" --no-service >/dev/null 2>&1
SUDO_USER=nobody-here bash "$src/scripts/uninstall.sh" >/dev/null 2>&1 && [ ! -e "$HOME/.keepalive-proxy" ] \
  && ok "default state folder resolves from HOME when not root" || bad "default state folder resolves from HOME when not root"

# Bad arguments fail.
bash "$src/scripts/install.sh" --port 99999 --dry-run >/dev/null 2>&1 \
  && bad "bad port refused" || ok "bad port refused"

# The scripts hold no user name or home path.
user=$(id -un)
leak=0
for f in "$src"/scripts/install.sh "$src"/scripts/uninstall.sh; do
  if grep -nE "/Us""ers/|/ho""me/" "$f"; then leak=1; fi
  if [ -n "$user" ] && grep -nwF -- "$user" "$f"; then leak=1; fi
  if [ -n "${real_home:-}" ] && grep -nF -- "$real_home" "$f"; then leak=1; fi
done
[ "$leak" = 0 ] && ok "scripts hold no user name or home path" || bad "scripts hold no user name or home path"

exit "$fail"
