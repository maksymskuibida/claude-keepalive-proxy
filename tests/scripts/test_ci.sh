#!/usr/bin/env bash
# Checks that the CI workflow contains every required step.
set -u
cd "$(dirname "$0")/../.." || exit 1
f=.github/workflows/ci.yml
fail=0
need() {
  if ! grep -Eq -- "$1" "$f"; then
    echo "FAIL: $2 (missing /$1/ in $f)"
    fail=1
  else
    echo "ok: $2"
  fi
}
[ -f "$f" ] || { echo "FAIL: $f missing"; exit 1; }
need '^[[:space:]]*pull_request:' "runs on pull requests"
need '^[[:space:]]*push:' "runs on push"
need 'branches:[[:space:]]*\[main\]' "push limited to main"
need 'runs-on:[[:space:]]*macos' "macOS runner"
need 'run:[[:space:]]*cargo build' "cargo build"
need 'run:[[:space:]]*cargo fmt --check' "cargo fmt --check"
need 'run:[[:space:]]*cargo clippy .*-D warnings' "clippy denies warnings"
need 'run:[[:space:]]*cargo test' "cargo test"
need 'tests/scripts/test_\*\.sh' "shell tests under tests/scripts"
need 'run:[[:space:]]*bash scripts/audit\.sh --range HEAD[[:space:]]*$' "leak audit over full history (--range HEAD)"
need 'fetch-depth:[[:space:]]*0' "full history checkout"
exit $fail
