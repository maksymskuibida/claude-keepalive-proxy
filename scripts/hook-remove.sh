#!/usr/bin/env bash
# Removes the pre-push hook installed by hook-install.sh, and only that one.
set -eu
root=$(git rev-parse --show-toplevel)
hook=$(git -C "$root" rev-parse --path-format=absolute --git-path hooks/pre-push)
marker="# keepalive-proxy audit hook"
if [ -e "$hook" ] && grep -qF "$marker" "$hook"; then
  rm -f "$hook"
  echo "hook-remove: removed $hook"
else
  echo "hook-remove: no audit hook installed; nothing to do"
fi
