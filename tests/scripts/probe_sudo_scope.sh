#!/usr/bin/env bash
# Probe: under sudo without --system, install.sh and uninstall.sh must run the user-scope
# service step as the same account; otherwise uninstall unloads in a different launchd
# domain than install loaded (TH-4: a running service left behind).
set -u
src=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
unset KEEPALIVE_PROXY_HOME KEEPALIVE_PROXY_LABEL KEEPALIVE_PROXY_PORT
me=$(/usr/bin/id -un)
fake=$work/fakebin; mkdir -p "$fake"; log=$work/log
# Pretend to be root: id -u prints 0; sudo -u U -H CMD runs CMD with RUN_AS=U.
printf '#!/bin/sh\nif [ "$1" = -u ]; then echo 0; else exec /usr/bin/id "$@"; fi\n' > "$fake/id"
printf '#!/bin/sh\nu=root\nwhile [ $# -gt 0 ]; do case "$1" in -u) u=$2; shift 2;; -H) shift;; *) break;; esac; done\nRUN_AS=$u exec "$@"\n' > "$fake/sudo"
chmod 755 "$fake/id" "$fake/sudo"
stub=$work/stub
printf '#!/bin/sh\necho "${RUN_AS:-root} $*" >> "%s"\n' "$log" > "$stub"; chmod 755 "$stub"
st=$work/state
PATH=$fake:$PATH SUDO_USER=$me bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.a >/dev/null 2>&1 || { echo "install failed"; exit 1; }
PATH=$fake:$PATH SUDO_USER=$me bash "$src/scripts/uninstall.sh" --home "$st" >/dev/null 2>&1 || { echo "uninstall failed"; exit 1; }
cat "$log"
inst=$(grep " service install" "$log" | cut -d' ' -f1)
uninst=$(grep " service uninstall" "$log" | cut -d' ' -f1)
[ -n "$inst" ] && [ "$inst" = "$uninst" ] || { echo "FAIL: user service installed as '$inst' but unloaded as '$uninst'"; exit 1; }
echo ok
