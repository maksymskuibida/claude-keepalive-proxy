#!/usr/bin/env bash
# Probe: the default deny-list (<git dir>/audit-denylist) must apply when the audit
# runs, or the pre-push hook fires, from a linked worktree of the same repository.
set -u
src=$(cd "$(dirname "$0")/../../.." && pwd)
work=$(mktemp -d); trap 'rm -rf "$work"' EXIT
export HOME="$work/h"; mkdir -p "$HOME"; export USER=zebulonq
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null; unset AUDIT_DENYLIST
r="$work/main"; mkdir -p "$r/scripts"; cp "$src"/scripts/*.sh "$r/scripts/"
git -C "$r" init -q; git -C "$r" config user.name T; git -C "$r" config user.email tester
echo clean > "$r/f.txt"; git -C "$r" add -A; git -C "$r" commit -q -m base
echo Bluefin > "$r/.git/audit-denylist"
git -C "$r" worktree add -q "$work/wt" -b wt
echo "codename Bluefin" > "$work/wt/g.txt"; git -C "$work/wt" add -A; git -C "$work/wt" commit -q -m g
(cd "$work/wt" && bash scripts/audit.sh >/dev/null 2>&1); rc=$?
echo "audit from worktree rc=$rc (expected 1)"
[ "$rc" -eq 1 ]
