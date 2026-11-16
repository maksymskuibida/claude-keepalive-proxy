#!/usr/bin/env bash
# Probe: install with label A, reinstall with label B, then uninstall.
# Every service the installs loaded must be unloaded (TH-4: no running service left behind).
set -u
src=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
unset KEEPALIVE_PROXY_HOME KEEPALIVE_PROXY_LABEL KEEPALIVE_PROXY_PORT
stub=$work/stub; log=$work/log
printf '#!/bin/sh\necho "$@" >> "%s"\n' "$log" > "$stub"; chmod 755 "$stub"
st=$work/state
bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.a >/dev/null 2>&1 || exit 1
bash "$src/scripts/install.sh" --home "$st" --bin "$stub" --label local.b >/dev/null 2>&1 || exit 1
bash "$src/scripts/uninstall.sh" --home "$st" >/dev/null 2>&1 || exit 1
cat "$log"
grep -qx "service uninstall --label local.a" "$log" || { echo "FAIL: local.a was loaded but never unloaded"; exit 1; }
grep -qx "service uninstall --label local.b" "$log" || { echo "FAIL: local.b never unloaded"; exit 1; }
echo ok
