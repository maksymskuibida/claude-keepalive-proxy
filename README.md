# keepalive-proxy

A tiny local HTTP CONNECT proxy that keeps long-lived HTTPS connections healthy on networks that silently drop idle connections. It is written in Rust, has no dependencies, and is aimed at Claude Code and similar clients.

**Platform: macOS only in this version** (it uses launchd for the service). Other systems are not supported yet.

## The problem

Many mobile and home networks sit behind carrier-grade NAT (CGNAT): one shared address translator run by the provider. To save memory, the translator forgets a connection after it has been quiet for a while, sometimes only a minute or two. It does not tell either side. Your computer still believes the connection is open, and so does the server.

A client such as Claude Code often holds a connection open while it waits for a long answer, with no traffic in between. When the translator forgets the connection, the next packet goes nowhere. The client waits for a reply that will never come, and the program appears to hang for many minutes until the operating system finally gives up.

## What the proxy does

The client talks to the proxy on `127.0.0.1`, and the proxy opens the real connection. On both legs of every tunnel the proxy turns on aggressive TCP keepalive probes. Small probe packets keep the translator's memory of the connection fresh, and if the path is really dead the probes fail. A dead path is cut within about 45 seconds, so the client sees an error at once and can retry, instead of hanging.

## Install

Clone the repository, then run the install script from the clone. It needs a Rust toolchain (`cargo`).

```sh
git clone <repository-url> keepalive-proxy
cd keepalive-proxy
scripts/install.sh
```

The script builds a release binary, copies it into the state folder (default `~/.keepalive-proxy`), writes a manifest of everything it created, and installs a per-user launchd service that listens on `127.0.0.1:18080`.

Flags:

| Flag | Meaning |
| --- | --- |
| `--home DIR` | state folder (default `$KEEPALIVE_PROXY_HOME`, else `~/.keepalive-proxy`) |
| `--label NAME` | service label |
| `--port N` | port the service listens on (default 18080) |
| `--system` | install the system daemon instead; run the whole script under `sudo` |
| `--no-service` | build and copy only; do not touch launchd |
| `--dry-run` | print what would be done and change nothing |
| `--bin FILE` | use a prebuilt binary instead of building |

Try `scripts/install.sh --dry-run` first to see the plan.

## Use

Point Claude Code at the proxy with one command, and undo it with one command:

```sh
keepalive-proxy on       # sets HTTPS_PROXY and NO_PROXY in Claude Code's settings file
keepalive-proxy off      # removes exactly those two entries
keepalive-proxy status   # shows what is set now
```

The installed binary is at `<state folder>/bin/keepalive-proxy`, by default `~/.keepalive-proxy/bin/keepalive-proxy`. The install script does not put it on your `PATH`. Call it by its full path (for example `~/.keepalive-proxy/bin/keepalive-proxy on`) or add its folder to `PATH` (`export PATH="$HOME/.keepalive-proxy/bin:$PATH"`). `on` accepts `--port N`, `--no-proxy HOSTS` to add hosts to the bypass list, and `--force` to overwrite an existing `HTTPS_PROXY` or `NO_PROXY` that differs. The settings file is `~/.claude/settings.json`, or the file named by `CLAUDE_SETTINGS`. A file that is not valid JSON is never touched. Restart Claude Code after changing it.

### Service commands

```sh
keepalive-proxy service install   [--system] [--dry-run] [--label NAME]
keepalive-proxy service uninstall [--system] [--dry-run] [--label NAME]
keepalive-proxy service status    [--system] [--dry-run] [--label NAME]
```

`keepalive-proxy serve` (also what plain `keepalive-proxy` does) runs the proxy in the foreground. The port comes from `KEEPALIVE_PROXY_PORT`, default 18080.

### Report

```sh
keepalive-proxy report [LOGFILE]
```

Reads the proxy log, which is `$KEEPALIVE_PROXY_HOME/proxy.log` when that variable is set and `~/.keepalive-proxy/proxy.log` otherwise. After an install with `--home DIR`, pass the log path to `report` (`report DIR/proxy.log`) or set `KEEPALIVE_PROXY_HOME` to `DIR`. It prints the number of tunnels, how many `DEADPATH` events were cut, and the longest silence a tunnel survived.

## Undo everything

```sh
keepalive-proxy off
scripts/uninstall.sh
```

`scripts/uninstall.sh` unloads the service and removes only the paths listed in the install manifest. It takes `--home DIR`, `--label NAME` and `--dry-run`. For a system install, run it under `sudo`.

## Honest limits

- **NO_PROXY is a deny list only.** It names hosts that skip the proxy; it cannot say "use the proxy only for these hosts". Everything not listed goes through the proxy.
- **Browser tools bypass the proxy.** Browser-based or separately launched tools that do not read `HTTPS_PROXY` from Claude Code's settings are not helped.
- **A down connection cannot be fixed.** The proxy keeps a healthy path alive and cuts a dead one quickly. It cannot revive a network that is really offline.
- **macOS only** in this version.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
