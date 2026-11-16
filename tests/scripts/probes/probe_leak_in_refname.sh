#!/usr/bin/env bash
# Probe: pushing a branch whose name holds the user name must be blocked by the hook.
set -u
src=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
export HOME="$work/h"; mkdir -p "$HOME"; export USER=zebulonq
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null; unset AUDIT_DENYLIST
r="$work/r"; mkdir -p "$r/scripts"; cp "$src"/scripts/*.sh "$r/scripts/"
git -C "$r" init -q; git -C "$r" config user.name T; git -C "$r" config user.email tester
git init -q --bare "$work/public.git"; git -C "$r" remote add public "$work/public.git"
echo clean > "$r/f.txt"; git -C "$r" add -A; git -C "$r" commit -q -m base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
git -C "$r" push -q public HEAD:refs/heads/main 2>/dev/null || { echo "setup push blocked"; exit 3; }
git -C "$r" push -q public HEAD:refs/heads/zebulonq/wip 2>/dev/null; rc=$?
echo "push of user-named branch rc=$rc (expected non-zero: blocked)"
[ "$rc" -ne 0 ]
