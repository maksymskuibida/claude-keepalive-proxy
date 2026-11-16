#!/usr/bin/env bash
# Checks that README.md carries each required statement.
set -u
src=$(cd "$(dirname "$0")/../.." && pwd)
readme="$src/README.md"
fail=0
need() { # description, pattern
  if grep -Eiq -- "$2" "$readme"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi
}
need "explains carrier-grade NAT" 'carrier-grade NAT \(CGNAT\)'
need "NAT forgets quiet connections" 'translator forgets a connection after it has been quiet'
need "idle connections hang the client" 'program appears to hang'
need "about 45 seconds" 'dead path is cut within about 45 seconds'
need "clone and install script" '^scripts/install\.sh$'
need "install flag --no-service" '`--no-service`'
need "install flag --dry-run" '`--dry-run`'
need "install flag --home" '`--home DIR`'
need "install flag --label" '`--label NAME`'
need "install flag --port" '`--port N`'
need "install flag --system" '`--system`'
need "install flag --bin" '`--bin FILE`'
need "enable command" '^keepalive-proxy on '
need "disable command" '^keepalive-proxy off '
need "status command" '^keepalive-proxy status '
need "service install command" '^keepalive-proxy service install '
need "service uninstall command" '^keepalive-proxy service uninstall '
need "service status command" '^keepalive-proxy service status '
need "serve command" '`keepalive-proxy serve`'
need "report command" '^keepalive-proxy report \[LOGFILE\]$'
need "uninstall script" '^scripts/uninstall\.sh$'
need "binary location" '`<state folder>/bin/keepalive-proxy`, by default `~/\.keepalive-proxy/bin/keepalive-proxy`'
need "binary not on PATH" 'does not put it on your `PATH`'
need "how to call the binary" 'full path.*add its folder to `PATH`'
need "report log from KEEPALIVE_PROXY_HOME" '`\$KEEPALIVE_PROXY_HOME/proxy\.log` when that variable is set'
need "report after --home install" 'After an install with `--home DIR`, pass the log path to `report`.*set `KEEPALIVE_PROXY_HOME`'
need "limit: deny-list-only NO_PROXY" '\*\*NO_PROXY is a deny list only\.\*\*'
need "limit: browser tools bypass" '\*\*Browser tools bypass the proxy\.\*\*'
need "limit: down connection" '\*\*A down connection cannot be fixed\.\*\*'
need "macOS only" '\*\*Platform: macOS only in this version\*\*'
need "Apache-2.0" '^Apache-2\.0\. See \[LICENSE\]'
if grep -Eq '/Users/|/home/|@[A-Za-z0-9-]+\.[a-z]' "$readme"; then echo "FAIL private text in README"; fail=1; else echo "ok   no private text"; fi
exit $fail
