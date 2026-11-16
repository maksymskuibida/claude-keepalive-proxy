#!/usr/bin/env bash
# Leak audit: scans tracked files and the commits about to be pushed
# (messages, author and committer emails, added lines) for personal data.
#
# Usage: scripts/audit.sh [--pre-push [<remote>] | --range <rev-range>]
#   --pre-push       read the pre-push hook's ref lines from stdin to pick the commits;
#                    <remote> is the hook's first argument: only commits already on
#                    that remote are skipped (without it, any remote counts)
#   --range A..B     audit the commits in that range, same as pre-push (for CI,
#                    e.g. --range "$BASE_SHA..$HEAD_SHA"; the base must be fetched)
#   default          audit every commit not yet on any remote, and every local
#                    annotated tag (it cannot tell offline which are pushed)
# For every selected commit the message, the author and committer headers and
# every added line of its diff and the paths it adds are scanned,
# as are file paths and pushed (or, by default, local) ref names, so a leak added and later removed
# inside the range is still found. Content hits read
# "commit <sha>:<file>:<line>: [kind] <text>"; the line is the
# line in that commit's version of the file.
#
# Environment:
#   AUDIT_DENYLIST  file with one forbidden string per line
#                   (default: <common git dir>/audit-denylist, never tracked;
#                   the same file applies from every linked worktree)
#
# Limits (accepted): UTF-16 files are not scanned as text; symlink targets in the
# tree scan are not read; deny-lists with CRLF line endings do not match;
# setsockopt failures in the proxy are ignored.
#
# Exit status: 0 clean, 1 at least one hit, 2 usage or environment error.
set -u
# Byte-wise scanning: a UTF-8 locale makes tr/grep stop at the first invalid byte.
export LC_ALL=C

usage="usage: audit.sh [--pre-push [<remote>] | --range <rev-range>]"
mode=default
range=
remote=
case "${1:-}" in
  "") ;;
  --pre-push)
    # git passes the remote name, then its URL; only the name is used.
    mode=pre-push; remote=${2:-} ;;
  --range)
    [ -n "${2:-}" ] && [ $# -eq 2 ] || { echo "$usage" >&2; exit 2; }
    mode=range; range=$2 ;;
  *) echo "$usage" >&2; exit 2 ;;
esac

root=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "audit: not in a git repository" >&2; exit 2; }
cd "$root" || exit 2
gitdir=$(git rev-parse --path-format=absolute --git-common-dir) || exit 2
denylist=${AUDIT_DENYLIST:-$gitdir/audit-denylist}

tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
hits=$tmp/hits
: > "$hits"
failed=$tmp/failed
: > "$failed"
# A scan step that fails must never read as clean.
fail_scan() { echo "$1" >> "$failed"; }

# Fixed strings: home path, deny-list lines. The user name is a case-insensitive
# regex whose boundaries exclude only ASCII letters (so name_96 and name2 match,
# a longer word of letters does not).
fixed=$tmp/fixed
: > "$fixed"
user=${USER:-$(id -un 2>/dev/null || true)}
case "$user" in
  ""|root|runner|ubuntu|admin|nobody) ;;
  *) [ "${#user}" -ge 3 ] && printf '(^|[^A-Za-z])%s([^A-Za-z]|$)\n' \
       "$(printf '%s' "$user" | sed 's/[][\\.*^$(){}?+|]/\\&/g')" > "$tmp/user" ;;
esac
home=${HOME:-}
[ "${#home}" -gt 1 ] && printf '%s\n' "$home" >> "$fixed"
if [ -f "$denylist" ]; then
  grep -v '^[[:space:]]*$' "$denylist" >> "$fixed" || true
fi
[ -f "$tmp/user" ] || : > "$tmp/user"

# Patterns are assembled from pieces so this file does not match itself.
home_re='(/Us''ers/|/ho''me/|[A-Za-z]:\\Us''ers\\)[A-Za-z0-9._-]+'
email_re='[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}'
link_re='(https?|file|ssh)://(local''host|127\.|0\.0\.0\.0|10\.[0-9]+\.|192\.168\.|172\.(1[6-9]|2[0-9]|3[01])\.|[^/ ]*\.(local|internal|corp|lan)([:/ ]|$)|[^/ ]*@)|file:/''//'

# tag <kind>: turns "N:text" grep output into "label:N: [kind] text".
# The label and kind travel in the environment, never inside a program text,
# so no file name can break or hide a hit.
tag() {
  KIND=$1 awk 'BEGIN { m = ENVIRON["MAP"]; if (m != "") { k = 0; while ((getline l < m) > 0) map[++k] = l } }
    { i = index($0, ":"); n = substr($0, 1, i - 1); if (n in map) n = map[n]
      if (ENVIRON["NOLN"] != "") printf "%s: [%s] %s\n", ENVIRON["LABEL"], ENVIRON["KIND"], substr($0, i + 1, 200)
      else printf "%s:%s: [%s] %s\n", ENVIRON["LABEL"], n, ENVIRON["KIND"], substr($0, i + 1, 200) }'
}

# scan <label>: reads text on stdin, appends "label:line: [kind] text" to hits.
scan() {
  export LABEL=$1
  export MAP=${2:-}
  export NOLN=${3:-}
  tr '\0' ' ' > "$tmp/in" || fail_scan "tr failed on $LABEL"
  grep -anF -f "$fixed" "$tmp/in" 2>/dev/null | tag home-or-denylist >> "$hits"
  if [ -s "$tmp/user" ]; then
    grep -aniE -f "$tmp/user" "$tmp/in" 2>/dev/null | tag user-name >> "$hits"
  fi
  grep -anE "$home_re" "$tmp/in" 2>/dev/null | tag home-path >> "$hits"
  grep -anoE "$email_re" "$tmp/in" 2>/dev/null | grep -viE ':[^@:]*@users\.noreply\.github\.com$|:no-?reply@[^@]*$' | tag email >> "$hits"
  grep -anE "$link_re" "$tmp/in" 2>/dev/null | tag private-link >> "$hits"
}

# scan_name <kind-of-name> <name>: scans a file path or ref name, reported
# as "<kind-of-name> <name>: [kind] text" (no line number).
scan_name() {
  printf '%s\n' "$2" | scan "$1 $2" "" 1
}

# 1. Tracked files: their names, then their contents.
while IFS= read -r -d '' f; do
  scan_name path "$f"
  [ -f "$f" ] || continue
  scan "$f" < "$f"
done < <(git ls-files -z)

# 2. Commits about to be pushed.
zero=0000000000000000000000000000000000000000
commits=$tmp/commits
: > "$commits"
tags=$tmp/tags
: > "$tags"
# note_tags <rev>: records the annotated tag objects on the way to a commit.
note_tags() {
  local o
  o=$(git rev-parse -q --verify "$1" 2>/dev/null) || return 0
  while [ "$(git cat-file -t "$o" 2>/dev/null)" = tag ]; do
    echo "$o" >> "$tags"
    o=$(git cat-file tag "$o" | sed -n 's/^object //p;/^$/q' | head -n 1)
    [ -n "$o" ] || break
  done
}
if [ -n "$remote" ]; then
  not_remotes="--remotes=$remote"
else
  not_remotes="--remotes"
fi
if [ "$mode" = pre-push ]; then
  while read -r _lref lsha rref rsha; do
    [ -n "${lsha:-}" ] || continue
    [ "$lsha" = "$zero" ] && continue
    [ -n "${rref:-}" ] && scan_name ref "$rref"
    note_tags "$lsha"
    if [ "$rsha" = "$zero" ]; then
      git rev-list "$lsha" --not "$not_remotes" >> "$commits" 2>/dev/null
    else
      git rev-list "$rsha..$lsha" >> "$commits" 2>/dev/null \
        || git rev-list "$lsha" --not "$not_remotes" >> "$commits" 2>/dev/null
    fi
  done
elif [ "$mode" = range ]; then
  git rev-list "$range" >> "$commits" 2>/dev/null || { echo "audit: bad range: $range" >&2; exit 2; }
  # Only the included end: the base (before ..) is excluded from the range.
  case "$range" in
    *..*) end=${range##*..}; [ -n "$end" ] || end=HEAD ;;
    *) end=${range#^} ;;
  esac
  note_tags "$end"
else
  git rev-list HEAD --not --remotes >> "$commits" 2>/dev/null
  while IFS= read -r rn; do
    [ -n "$rn" ] && scan_name ref "$rn"
  done < <(git for-each-ref --format='%(refname:short)' refs/heads refs/tags)
  while read -r tobj ttype; do
    [ "$ttype" = tag ] && echo "$tobj" >> "$tags"
  done < <(git for-each-ref --format='%(objectname) %(objecttype)' refs/tags)
fi
sort -u "$tags" | while read -r t; do
  [ -n "$t" ] || continue
  git cat-file tag "$t" | scan "tag $(printf '%.10s' "$t")"
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail_scan "git cat-file failed on tag $t"
done
sort -u "$commits" | while read -r c; do
  [ -n "$c" ] || continue
  short=$(printf '%.10s' "$c")
  git log -1 --format='%B' "$c" | scan "commit $short message"
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail_scan "git log failed on $c"
  git log -1 --format='author: %an <%ae>%ncommitter: %cn <%ce>' "$c" | scan "commit $short header"
  [ "${PIPESTATUS[0]}" -eq 0 ] || fail_scan "git log failed on $c"
  : > "$tmp/map"
  # A merge shows no diff by itself, so it is diffed against its first parent:
  # this finds lines only the merge adds (and also re-scans merged-in lines).
  if [ "$(git rev-list --parents -n 1 "$c" | wc -w | tr -d ' ')" -gt 2 ]; then
    difftree() { git -c core.quotepath=off diff-tree -r -p -U0 --text --no-color --no-ext-diff "$1^1" "$1"; }
  else
    difftree() { git -c core.quotepath=off diff-tree -r --root -p -U0 --text --no-color --no-ext-diff "$1"; }
  fi
  if [ "$(git rev-list --parents -n 1 "$c" | wc -w | tr -d ' ')" -gt 2 ]; then
    git -c core.quotepath=off diff-tree -r -z --name-only --diff-filter=AR "$c^1" "$c" > "$tmp/paths"
  else
    git -c core.quotepath=off diff-tree -r -z --root --no-commit-id --name-only --diff-filter=AR "$c" > "$tmp/paths"
  fi || fail_scan "path listing failed on $c"
  while IFS= read -r -d '' pf; do scan_name "commit $short path" "$pf"; done < "$tmp/paths"
  difftree "$c" | tr '\0' ' ' | awk -v mapf="$tmp/map" '
    /^diff --git / { hdr = 1; next }
    hdr && /^\+\+\+ / { f = substr($0, 5); sub(/^b\//, "", f); sub(/\t.*$/, "", f); next }
    /^@@ / { hdr = 0; if (match($0, /\+[0-9]+/)) n = substr($0, RSTART + 1, RLENGTH - 1) + 0; next }
    !hdr && /^\+/ { print f ":" n > mapf; print substr($0, 2); n++ }
  ' | scan "commit $short" "$tmp/map"
  for st in "${PIPESTATUS[@]}"; do
    [ "$st" -eq 0 ] || { fail_scan "diff scan failed on $c"; break; }
  done
done

if [ -s "$failed" ]; then
  cat "$hits"
  sed 's/^/audit: /' "$failed" >&2
  echo "audit: scan incomplete, not clean" >&2
  exit 2
fi

if [ -s "$hits" ]; then
  cat "$hits"
  echo "audit: $(wc -l < "$hits" | tr -d ' ') hit(s) found" >&2
  exit 1
fi
echo "audit: clean"
exit 0
