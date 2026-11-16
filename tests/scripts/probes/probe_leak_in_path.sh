#!/usr/bin/env bash
# Probe: a leak that sits only in a tracked file's path (not its content) must be
# found by the tree scan and must block the pre-push hook.
set -u
src=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
export HOME="$work/h"; mkdir -p "$HOME"; export USER=zebulonq
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null; unset AUDIT_DENYLIST
r="$work/r"; mkdir -p "$r/scripts"; cp "$src"/scripts/*.sh "$r/scripts/"
git -C "$r" init -q; git -C "$r" config user.name T; git -C "$r" config user.email tester
git init -q --bare "$work/public.git"; git -C "$r" remote add public "$work/public.git"
echo clean > "$r/f.txt"; git -C "$r" add -A; git -C "$r" commit -q -m base
git -C "$r" push -q --no-verify public HEAD:refs/heads/main
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
mkdir -p "$r/notes"; echo "plain text" > "$r/notes/zebulonq-todo.txt"
git -C "$r" add -A; git -C "$r" commit -q -m notes
(cd "$r" && bash scripts/audit.sh >/dev/null 2>&1); arc=$?
git -C "$r" push -q public HEAD:refs/heads/main 2>/dev/null; prc=$?
echo "audit rc=$arc push rc=$prc (expected both non-zero)"
[ "$arc" -ne 0 ] && [ "$prc" -ne 0 ]
