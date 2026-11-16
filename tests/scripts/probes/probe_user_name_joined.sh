#!/usr/bin/env bash
# Probe: the user name joined to an underscore or digit (a handle such as <user>_96,
# or <user>2) in content, a commit message or a path must be found by the audit and
# block the hook. Expected: audit exits non-zero naming the content, message and path
# hits, the push is blocked; this script then exits 0.
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
echo "maintainer: Zebulonq_96 and zebulonq2" > "$r/f.txt"; echo x > "$r/zebulonq_notes.txt"
git -C "$r" add -A; git -C "$r" commit -q -m "thanks zebulonq_96"
out=$(cd "$r" && bash scripts/audit.sh 2>&1); arc=$?
git -C "$r" push -q public HEAD:refs/heads/main 2>/dev/null; prc=$?
echo "$out"
[ "$arc" -ne 0 ] && [ "$prc" -ne 0 ] || { echo "FAIL: audit rc=$arc push rc=$prc"; exit 1; }
grep -q 'f.txt:1: \[user-name\]' <<<"$out" || { echo "FAIL: no content hit"; exit 1; }
grep -q 'message:1: \[user-name\]' <<<"$out" || { echo "FAIL: no message hit"; exit 1; }
grep -q 'path zebulonq_notes.txt: \[user-name\]' <<<"$out" || { echo "FAIL: no path hit"; exit 1; }
exit 0
