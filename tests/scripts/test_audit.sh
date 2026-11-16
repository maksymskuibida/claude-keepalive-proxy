#!/usr/bin/env bash
# Tests for scripts/audit.sh and the hook scripts, in temp git repositories.
set -u
src=$(cd "$(dirname "$0")/../.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }

real_home=$HOME; real_user=${USER:-}
export HOME="$work/fakehome"; mkdir -p "$HOME"
export USER=zebulonq
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
unset AUDIT_DENYLIST

newrepo() {
  r="$work/$1"; mkdir -p "$r/scripts"
  cp "$src"/scripts/*.sh "$r/scripts/"
  git -C "$r" init -q
  git -C "$r" config user.name Tester
  git -C "$r" config user.email 'tester@users.noreply.github.com'
  echo "$r"
}
commit() { git -C "$1" add -A && git -C "$1" commit -q -m "${2:-change}"; }
audit() { (cd "$1" && bash scripts/audit.sh "${@:2}"); }

# Clean repo passes.
r=$(newrepo clean); echo "hello world" > "$r/a.txt"; commit "$r"
out=$(audit "$r") && ok "clean tree passes" || { bad "clean tree passes"; echo "$out"; }

# Planted leaks in tracked files are found with file and line.
leaks=(
  "who is zebulonq here"
  "path $HOME/project"
  "see /Us""ers/someone/code"
  "mail bob@exam""ple.com now"
  "http://local""host:8080/x"
  "http://192.168"".1.5/admin"
)
i=0
for l in "${leaks[@]}"; do
  i=$((i+1)); r=$(newrepo "leak$i")
  printf 'line one\n%s\n' "$l" > "$r/f.txt"; commit "$r"
  out=$(audit "$r"); rc=$?
  if [ $rc -ne 0 ] && grep -q '^f.txt:2:' <<<"$out"; then ok "leak $i found at f.txt:2"; else bad "leak $i ($l) rc=$rc out=$out"; fi
done

# noreply email is allowed.
r=$(newrepo noreply); echo "x 1234+me@users.noreply.github.com" > "$r/f.txt"; commit "$r"
audit "$r" >/dev/null && ok "noreply email passes" || bad "noreply email passes"

# Leak in a commit message only.
r=$(newrepo msg); echo clean > "$r/f.txt"; commit "$r" "fix for zebulonq machine"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -q 'message:1:' <<<"$out" && ok "commit message leak found" || bad "commit message leak rc=$rc"

# Leak in author email.
r=$(newrepo author); echo clean > "$r/f.txt"
git -C "$r" add -A; git -C "$r" -c user.email=real@priv""ate.org commit -q -m ok
audit "$r" >/dev/null && bad "author email leak" || ok "author email leak found"

# Deny-list outside the tree, default location inside .git.
r=$(newrepo deny); echo "project codename Bluefin" > "$r/f.txt"; commit "$r"
audit "$r" >/dev/null && ok "no deny-list: passes" || bad "no deny-list: passes"
echo "Bluefin" > "$r/.git/audit-denylist"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -q '^f.txt:1:' <<<"$out" && ok "default deny-list hit" || bad "default deny-list hit"
rm "$r/.git/audit-denylist"
echo "Bluefin" > "$work/other-list"
AUDIT_DENYLIST="$work/other-list" audit "$r" >/dev/null && bad "env deny-list" || ok "env deny-list hit"

# Already-pushed commits (on a remote) are not rescanned for messages.
r=$(newrepo pushed); echo clean > "$r/f.txt"; commit "$r" "zebulonq msg"
git -C "$r" update-ref refs/remotes/origin/main HEAD
audit "$r" >/dev/null && ok "commits already on remote skipped" || bad "commits already on remote skipped"

# Hook: install blocks a real push, remove restores.
r=$(newrepo hook); bare="$work/bare.git"; git init -q --bare "$bare"
git -C "$r" remote add origin "$bare"
echo clean > "$r/f.txt"; commit "$r"
printf '#!/bin/sh\necho foreign\n' > "$r/.git/hooks/pre-commit"; chmod +x "$r/.git/hooks/pre-commit"
before=$(cksum < "$r/.git/hooks/pre-commit")
(cd "$r" && bash scripts/hook-install.sh >/dev/null) && [ -x "$r/.git/hooks/pre-push" ] && ok "hook installed" || bad "hook installed"
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && ok "clean push allowed" || bad "clean push allowed"
echo "leak bob@exam""ple.com" > "$r/g.txt"; commit "$r" "add g"
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && bad "leaky push blocked" || ok "leaky push blocked"
[ -z "$(git -C "$bare" rev-parse -q --verify refs/heads/main | grep -x "$(git -C "$r" rev-parse HEAD)")" ] && ok "remote not updated" || bad "remote not updated"
(cd "$r" && bash scripts/hook-remove.sh >/dev/null); [ ! -e "$r/.git/hooks/pre-push" ] && ok "hook removed" || bad "hook removed"
[ "$before" = "$(cksum < "$r/.git/hooks/pre-commit")" ] && ok "foreign pre-commit untouched" || bad "foreign pre-commit untouched"

# Foreign pre-push hook is left alone by install and remove.
r=$(newrepo foreign)
printf '#!/bin/sh\necho mine\n' > "$r/.git/hooks/pre-push"; chmod +x "$r/.git/hooks/pre-push"
before=$(cksum < "$r/.git/hooks/pre-push")
(cd "$r" && bash scripts/hook-install.sh >/dev/null 2>&1) && bad "install refuses foreign pre-push" || ok "install refuses foreign pre-push"
(cd "$r" && bash scripts/hook-remove.sh >/dev/null 2>&1)
[ "$before" = "$(cksum < "$r/.git/hooks/pre-push")" ] && ok "foreign pre-push untouched" || bad "foreign pre-push untouched"

# F-6: a leak added then removed inside the pushed range is still found and blocks.
r=$(newrepo gone); bare2="$work/bare2.git"; git init -q --bare "$bare2"
git -C "$r" remote add origin "$bare2"
echo clean > "$r/f.txt"; commit "$r" base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && ok "base push allowed" || bad "base push allowed"
echo "mail bob@exam""ple.com" > "$r/h.txt"; commit "$r" "add h"
git -C "$r" rm -q h.txt; commit "$r" "drop h"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -q 'commit .*:h.txt:1: \[email\]' <<<"$out" && ok "removed leak found in history" || bad "removed leak found in history rc=$rc out=$out"
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && bad "push with removed leak blocked" || ok "push with removed leak blocked"
# A leak on a branch that is not checked out is found through the hook's ref lines.
git -C "$r" checkout -q -b side HEAD~2; echo "mail eve@exam""ple.com" > "$r/s.txt"; commit "$r" "side"
git -C "$r" checkout -q -
git -C "$r" reset -q --hard HEAD~2
git -C "$r" push -q origin side 2>/dev/null && bad "other-branch push blocked" || ok "other-branch push blocked"

# F-7: explicit range for CI, same scan as pre-push.
r=$(newrepo range); echo clean > "$r/f.txt"; commit "$r" base
base=$(git -C "$r" rev-parse HEAD)
echo clean2 > "$r/f.txt"; commit "$r" "oops zebulonq"
echo "x bob@exam""ple.com" > "$r/k.txt"; commit "$r" k; git -C "$r" rm -q k.txt; commit "$r" "rm k"
out=$(audit "$r" --range "$base..HEAD"); rc=$?
[ $rc -ne 0 ] && grep -q 'message:1:' <<<"$out" && grep -q ':k.txt:1:' <<<"$out" && ok "range scans messages and content" || bad "range rc=$rc out=$out"
audit "$r" --range "nonsense..HEAD" >/dev/null 2>&1; [ $? -eq 2 ] && ok "bad range exits 2" || bad "bad range exits 2"

# F-8: file names with | & and backslash do not hide hits.
r=$(newrepo meta); echo "see /Us""ers/bob/x" > "$r/a|b&c\\d.txt"; commit "$r"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -qF 'a|b&c' <<<"$out" && ok "metachar file name hit kept" || bad "metachar file name rc=$rc out=$out"

# F-9: lines added only by a merge commit (built with plumbing), removed later.
r=$(newrepo merge); bare3="$work/bare3.git"; git init -q --bare "$bare3"
git -C "$r" remote add origin "$bare3"
echo clean > "$r/f.txt"; commit "$r" base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && ok "merge base push allowed" || bad "merge base push allowed"
base=$(git -C "$r" rev-parse HEAD)
git -C "$r" checkout -q -b side; echo s > "$r/s.txt"; commit "$r" side
sidec=$(git -C "$r" rev-parse HEAD)
git -C "$r" checkout -q -; echo p > "$r/p.txt"; commit "$r" mainline
blob=$(echo "mail evil@exam""ple.com" | git -C "$r" hash-object -w --stdin)
idx="$work/merge-index"; rm -f "$idx"
GIT_INDEX_FILE=$idx git -C "$r" read-tree HEAD
GIT_INDEX_FILE=$idx git -C "$r" update-index --add --cacheinfo "100644,$blob,m.txt"
tree=$(GIT_INDEX_FILE=$idx git -C "$r" write-tree)
mc=$(git -C "$r" commit-tree "$tree" -p HEAD -p "$sidec" -m "merge side")
git -C "$r" reset -q --hard "$mc"
git -C "$r" rm -q m.txt; commit "$r" "drop m"
out=$(audit "$r" --range "$base..HEAD"); rc=$?
[ $rc -ne 0 ] && grep -q 'commit .*:m.txt:1: \[email\]' <<<"$out" && ok "merge-only leak found by --range" || bad "merge-only leak --range rc=$rc out=$out"
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && bad "merge-only leak push blocked" || ok "merge-only leak push blocked"

# Binary files: a leak next to a NUL byte is found, by file name, in the tree, in history and on push.
r=$(newrepo bin); bareb="$work/bareb.git"; git init -q --bare "$bareb"
git -C "$r" remote add origin "$bareb"
echo clean > "$r/f.txt"; commit "$r" base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null
printf 'artefact\0 mail bob@exam''ple.com\n' > "$r/blob.bin"; commit "$r" "add artefact"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -q '^blob.bin:' <<<"$out" && grep -q 'commit .*:blob.bin:' <<<"$out" && ok "binary file leak found" || bad "binary file leak rc=$rc out=$out"
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null && bad "binary leak push blocked" || ok "binary leak push blocked"

# Annotated tags: message and tagger are scanned on push and with --range.
r=$(newrepo tag); baret="$work/baret.git"; git init -q --bare "$baret"
git -C "$r" remote add origin "$baret"
echo clean > "$r/f.txt"; commit "$r" base
(cd "$r" && bash scripts/hook-install.sh >/dev/null)
git -C "$r" push -q origin HEAD:refs/heads/main 2>/dev/null
git -C "$r" tag -a v1 -m "built on zebulonq laptop"
git -C "$r" push -q origin v1 2>/dev/null && bad "tag message push blocked" || ok "tag message push blocked"
out=$(audit "$r" --range "v1"); rc=$?
[ $rc -ne 0 ] && grep -q '^tag .*user-name' <<<"$out" && ok "tag found by --range" || bad "tag --range rc=$rc out=$out"
git -C "$r" tag -d v1 >/dev/null
git -C "$r" -c user.email=real@priv''ate.org tag -a v2 -m "ok"
git -C "$r" push -q origin v2 2>/dev/null && bad "tagger email push blocked" || ok "tagger email push blocked"
git -C "$r" tag -d v2 >/dev/null
git -C "$r" tag -a v3 -m "fine"
git -C "$r" push -q origin v3 2>/dev/null && ok "clean tag push allowed" || bad "clean tag push allowed"

# noreply: only real noreply forms are exempt.
r=$(newrepo nr); printf 'a noreply@exam''ple.org\nb 42+x@users.noreply.github.com\n' > "$r/f.txt"; commit "$r"
audit "$r" >/dev/null && ok "real noreply forms pass" || bad "real noreply forms pass"
r=$(newrepo nr2); echo "contact noreply.fan@gma""il.com" > "$r/f.txt"; commit "$r"
out=$(audit "$r"); rc=$?
[ $rc -ne 0 ] && grep -q '^f.txt:1:' <<<"$out" && ok "noreply lookalike found" || bad "noreply lookalike rc=$rc"

# The real repository's own tracked files are clean.
if (cd "$src" && git rev-parse -q --verify HEAD >/dev/null 2>&1); then
  (cd "$src" && HOME=$real_home USER=$real_user bash scripts/audit.sh >/dev/null) && ok "this repo audits clean" || bad "this repo audits clean"
fi

exit $fail
