#!/usr/bin/env bash
# Leak audit: scans tracked files and the commits about to be pushed
# (messages, author and committer emails, added lines) for personal data.
#
# Usage: scripts/audit.sh [--pre-push | --range <rev-range>]
#   --pre-push       read the pre-push hook's ref lines from stdin to pick the commits
#   --range A..B     audit the commits in that range, same as pre-push (for CI,
#                    e.g. --range "$BASE_SHA..$HEAD_SHA"; the base must be fetched)
#   default          audit every commit not yet on any remote
# For every selected commit the message, the author and committer headers and
# every added line of its diff are scanned, so a leak added and later removed
# inside the range is still found. Content hits read
# "commit <sha>:<file>:<line>: [kind] <text>"; the line is the
# line in that commit's version of the file.
#
# Environment:
#   AUDIT_DENYLIST  file with one forbidden string per line
#                   (default: <git dir>/audit-denylist, never tracked)
#
# Exit status: 0 clean, 1 at least one hit, 2 usage or environment error.
set -u

usage="usage: audit.sh [--pre-push | --range <rev-range>]"
mode=default
range=
case "${1:-}" in
  "") ;;
  --pre-push) mode=pre-push ;;
  --range)
    [ -n "${2:-}" ] && [ $# -eq 2 ] || { echo "$usage" >&2; exit 2; }
    mode=range; range=$2 ;;
  *) echo "$usage" >&2; exit 2 ;;
esac

root=$(git rev-parse --show-toplevel 2>/dev/null) || { echo "audit: not in a git repository" >&2; exit 2; }
cd "$root" || exit 2
gitdir=$(git rev-parse --absolute-git-dir)
denylist=${AUDIT_DENYLIST:-$gitdir/audit-denylist}

tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
hits=$tmp/hits
: > "$hits"

# Fixed strings: current user, home path, deny-list lines.
fixed=$tmp/fixed
: > "$fixed"
user=${USER:-$(id -un 2>/dev/null || true)}
case "$user" in
  ""|root|runner|ubuntu|admin|nobody) ;;
  *) [ "${#user}" -ge 3 ] && printf '%s\n' "$user" > "$tmp/user" ;;
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
      printf "%s:%s: [%s] %s\n", ENVIRON["LABEL"], n, ENVIRON["KIND"], substr($0, i + 1) }'
}

# scan <label>: reads text on stdin, appends "label:line: [kind] text" to hits.
scan() {
  export LABEL=$1
  export MAP=${2:-}
  cat > "$tmp/in"
  grep -nF -f "$fixed" "$tmp/in" 2>/dev/null | tag home-or-denylist >> "$hits"
  if [ -s "$tmp/user" ]; then
    grep -nwiF -f "$tmp/user" "$tmp/in" 2>/dev/null | tag user-name >> "$hits"
  fi
  grep -nE "$home_re" "$tmp/in" 2>/dev/null | tag home-path >> "$hits"
  grep -noE "$email_re" "$tmp/in" 2>/dev/null | grep -vi 'noreply' | tag email >> "$hits"
  grep -nE "$link_re" "$tmp/in" 2>/dev/null | tag private-link >> "$hits"
}

# 1. Tracked files.
while IFS= read -r -d '' f; do
  [ -f "$f" ] || continue
  grep -qI '' "$f" 2>/dev/null || continue
  scan "$f" < "$f"
done < <(git ls-files -z)

# 2. Commits about to be pushed.
zero=0000000000000000000000000000000000000000
commits=$tmp/commits
: > "$commits"
if [ "$mode" = pre-push ]; then
  while read -r _lref lsha _rref rsha; do
    [ -n "${lsha:-}" ] || continue
    [ "$lsha" = "$zero" ] && continue
    if [ "$rsha" = "$zero" ]; then
      git rev-list "$lsha" --not --remotes >> "$commits" 2>/dev/null
    else
      git rev-list "$rsha..$lsha" >> "$commits" 2>/dev/null \
        || git rev-list "$lsha" --not --remotes >> "$commits" 2>/dev/null
    fi
  done
elif [ "$mode" = range ]; then
  git rev-list "$range" >> "$commits" 2>/dev/null || { echo "audit: bad range: $range" >&2; exit 2; }
else
  git rev-list HEAD --not --remotes >> "$commits" 2>/dev/null
fi
sort -u "$commits" | while read -r c; do
  [ -n "$c" ] || continue
  short=$(printf '%.10s' "$c")
  git log -1 --format='%B' "$c" | scan "commit $short message"
  git log -1 --format='author: %an <%ae>%ncommitter: %cn <%ce>' "$c" | scan "commit $short header"
  : > "$tmp/map"
  # A merge shows no diff by itself, so it is diffed against its first parent:
  # this finds lines only the merge adds (and also re-scans merged-in lines).
  if [ "$(git rev-list --parents -n 1 "$c" | wc -w | tr -d ' ')" -gt 2 ]; then
    difftree() { git -c core.quotepath=off diff-tree -r -p -U0 --no-color --no-ext-diff "$1^1" "$1"; }
  else
    difftree() { git -c core.quotepath=off diff-tree -r --root -p -U0 --no-color --no-ext-diff "$1"; }
  fi
  difftree "$c" | awk -v mapf="$tmp/map" '
    /^diff --git / { hdr = 1; next }
    hdr && /^\+\+\+ / { f = substr($0, 5); sub(/^b\//, "", f); sub(/\t.*$/, "", f); next }
    /^@@ / { hdr = 0; if (match($0, /\+[0-9]+/)) n = substr($0, RSTART + 1, RLENGTH - 1) + 0; next }
    !hdr && /^\+/ { print f ":" n > mapf; print substr($0, 2); n++ }
  ' | scan "commit $short" "$tmp/map"
done

if [ -s "$hits" ]; then
  cat "$hits"
  echo "audit: $(wc -l < "$hits" | tr -d ' ') hit(s) found" >&2
  exit 1
fi
echo "audit: clean"
exit 0
