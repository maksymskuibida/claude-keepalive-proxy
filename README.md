# claude-keepalive-proxy

A tiny local HTTP CONNECT proxy that keeps long-lived HTTPS connections alive on networks that silently drop idle connections (carrier-grade NAT, flaky routers). It sets aggressive TCP keepalive on both legs of every tunnel, so a dead path is cut within about 45 seconds instead of leaving the client hanging for minutes.

Status: baseline code is arriving in the first pull request. Not yet packaged or documented.

## Licence

Apache-2.0. See [LICENSE](LICENSE).
