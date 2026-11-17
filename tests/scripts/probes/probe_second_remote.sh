#!/usr/bin/env bash
# Probe: pushing a new branch to a public remote must scan its commits even when
# they are already on a different (private) remote.
set -u
src=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
export HOME="$work/h"; mkdir -p "$HOME"; export USER=zebulonq
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null; unset AUDIT_DENYLIST
r="$work/r"; mkdir -p "$r/scripts"; cp "$src"/scripts/*.sh "$r/scripts/"
git -C "$r" init -q; git -C "$r" config user.name T; git -C "$r" config user.email tester
git init -q --bare "$work/public.git"; git init -q --bare "$work/private.git"
git -C "$r" remote add public "$work/public.git"; git -C "$r" remote add private "$work/private.git"
echo clean > "$r/f.txt"; git -C "$r" add -A; git -C "$r" commit -q -m base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
echo Bluefin > "$r/.git/audit-denylist"; echo "codename Bluefin" > "$r/g.txt"; git -C "$r" add -A; git -C "$r" commit -q -m g
git -C "$r" rm -q g.txt; git -C "$r" commit -q -m rm
git -C "$r" push -q --no-verify private HEAD:refs/heads/main
git -C "$r" push -q public HEAD:refs/heads/main 2>/dev/null; rc=$?
echo "push to public rc=$rc (expected non-zero: blocked)"
[ "$rc" -ne 0 ]
