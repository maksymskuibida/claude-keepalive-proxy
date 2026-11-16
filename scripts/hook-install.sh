#!/usr/bin/env bash
# Installs the pre-push hook that runs scripts/audit.sh.
# Refuses to replace a pre-push hook it did not create; touches no other hook.
set -eu
root=$(git rev-parse --show-toplevel)
hook=$(git -C "$root" rev-parse --path-format=absolute --git-path hooks/pre-push)
marker="# keepalive-proxy audit hook"
if [ -e "$hook" ] && ! grep -qF "$marker" "$hook"; then
  echo "hook-install: a different pre-push hook exists at $hook; leaving it alone" >&2
  exit 1
fi
mkdir -p "$(dirname "$hook")"
cat > "$hook" <<HOOK
#!/usr/bin/env bash
$marker
exec "$root/scripts/audit.sh" --pre-push
HOOK
chmod +x "$hook"
echo "hook-install: installed $hook"
