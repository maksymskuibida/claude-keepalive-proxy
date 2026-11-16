#!/usr/bin/env bash
# Installs keepalive-proxy from a clone: builds it with cargo (release), copies the
# binary into the state folder, writes a manifest of what it created and installs
# the per-user service through the binary (`keepalive-proxy service install`).
#
# Usage: scripts/install.sh [--home DIR] [--label NAME] [--port N] [--system]
#                           [--no-service] [--dry-run] [--bin FILE]
#   --home DIR     state folder (default: $KEEPALIVE_PROXY_HOME, else ~/.keepalive-proxy)
#   --label NAME   service label (default: $KEEPALIVE_PROXY_LABEL, else the binary's own default)
#   --port N       listen port given to the service (default: $KEEPALIVE_PROXY_PORT, else the
#                  binary's own default)
#   --system       install the system daemon instead. Run the whole script under sudo: the
#                  build then runs as the invoking user ($SUDO_USER), the state folder
#                  defaults to that user's home (not root's) and the files it creates are
#                  handed to that user; only the service step runs as root. Without
#                  $SUDO_USER, a root run needs --home and --bin (a root build is refused).
#                  Without --system, a sudo run loads the per-user service as the invoking
#                  user (as uninstall.sh unloads it); a root run with no $SUDO_USER and no
#                  --system refuses to install the service.
#   --no-service   build and copy only; do not touch launchd
#   --dry-run      print what would be done; change nothing
#   --bin FILE     use this prebuilt binary instead of building with cargo
#
# The manifest (<home>/install-manifest) lists every path created, so
# scripts/uninstall.sh removes those and nothing else.
set -eu

usage="usage: install.sh [--home DIR] [--label NAME] [--port N] [--system] [--no-service] [--dry-run] [--bin FILE]"
die() { echo "install: $*" >&2; exit 1; }

home_dir=${KEEPALIVE_PROXY_HOME:-}
label=${KEEPALIVE_PROXY_LABEL:-}
port=${KEEPALIVE_PROXY_PORT:-}
system=0 no_service=0 dry=0 prebuilt=
while [ $# -gt 0 ]; do
  case "$1" in
    --home|--label|--port|--bin)
      [ $# -ge 2 ] || die "$1 needs a value ($usage)"
      case "$1" in
        --home) home_dir=$2 ;; --label) label=$2 ;; --port) port=$2 ;; --bin) prebuilt=$2 ;;
      esac
      shift 2 ;;
    --system) system=1; shift ;;
    --no-service) no_service=1; shift ;;
    --dry-run) dry=1; shift ;;
    -h|--help) echo "$usage"; exit 0 ;;
    *) die "unknown argument '$1' ($usage)" ;;
  esac
done

# Under sudo, act for the invoking user: build, state folder and file ownership.
run_user=
if [ "$(id -u)" = 0 ] && [ -n "${SUDO_USER:-}" ] && [ "$SUDO_USER" != root ]; then
  case "$SUDO_USER" in *[!A-Za-z0-9._-]*) die "unusable SUDO_USER '$SUDO_USER'" ;; esac
  run_user=$SUDO_USER
fi
if [ -z "$home_dir" ]; then
  if [ -n "$run_user" ]; then
    user_home=$(eval "echo ~$run_user")
    case "$user_home" in /*) ;; *) die "cannot find the home of $run_user; pass --home DIR" ;; esac
    home_dir=$user_home/.keepalive-proxy
  else
    [ -n "${HOME:-}" ] || die "HOME is not set; pass --home DIR"
    home_dir=$HOME/.keepalive-proxy
  fi
fi
case "$home_dir" in /*) ;; *) die "the state folder must be an absolute path: $home_dir" ;; esac
if [ -n "$port" ]; then
  case "$port" in *[!0-9]*) die "invalid port '$port'" ;; esac
  [ "$port" -ge 1 ] && [ "$port" -le 65535 ] || die "invalid port '$port'"
fi
if [ -n "$label" ]; then
  case "$label" in *[!A-Za-z0-9._-]*) die "invalid label '$label'" ;; esac
fi

root=$(cd "$(dirname "$0")/.." && pwd)
bin_dir=$home_dir/bin
bin=$bin_dir/keepalive-proxy
manifest=$home_dir/install-manifest

service_args=(service install)
[ "$system" = 1 ] && service_args+=(--system)
[ -n "$label" ] && service_args+=(--label "$label")

if [ "$dry" = 1 ]; then
  if [ -n "$prebuilt" ]; then echo "would use prebuilt binary $prebuilt"
  else echo "would run: cargo build --release (in $root)"; fi
  echo "would create $bin_dir and copy the binary to $bin"
  echo "would write $manifest"
  if [ "$no_service" = 1 ]; then echo "would skip the service (--no-service)"
  else echo "would run: $bin ${service_args[*]}${port:+ (with KEEPALIVE_PROXY_PORT=$port)}"; fi
  exit 0
fi

if [ -n "$prebuilt" ]; then
  src_bin=$prebuilt
else
  if [ "$(id -u)" = 0 ] && [ -z "$run_user" ]; then
    die "refusing to run cargo build as root; build as a normal user or pass --bin FILE"
  fi
  if [ -n "$run_user" ]; then
    (cd "$root" && sudo -u "$run_user" -H sh -c 'command -v cargo >/dev/null 2>&1 || exit 127; cargo build --release') \
      || die "cargo build failed (as $run_user; is cargo installed for that user?)"
  else
    command -v cargo >/dev/null 2>&1 || die "cargo not found; install the Rust toolchain first"
    (cd "$root" && cargo build --release) || die "cargo build failed"
  fi
  target=${CARGO_TARGET_DIR:-$root/target}
  src_bin=$target/release/keepalive-proxy
fi
[ -f "$src_bin" ] || die "binary not found: $src_bin"

# Refuse a foreign binary before creating anything.
existing_bin=0
if [ -e "$bin" ]; then
  # Replace only a binary an earlier install of ours recorded.
  grep -qxF "file bin/keepalive-proxy" "$manifest" 2>/dev/null \
    || die "$bin exists and was not installed by this script; leaving it alone"
  existing_bin=1
fi

# Record only what this run creates; anything already there is not ours to remove.
created=() new_paths=()
[ -d "$home_dir" ] || { mkdir -p "$home_dir"; created+=("dir ."); new_paths+=("$home_dir"); }
[ -d "$bin_dir" ] || { mkdir "$bin_dir"; created+=("dir bin"); new_paths+=("$bin_dir"); }
cp "$src_bin" "$bin.new" && chmod 755 "$bin.new" && mv -f "$bin.new" "$bin"
created+=("file bin/keepalive-proxy")
created+=("file install-manifest")

{
  echo "# keepalive-proxy install manifest: uninstall removes only what is listed here"
  # Keep entries from an earlier install that this run did not re-create.
  if [ -f "$manifest" ]; then
    # Service lines stay too, so uninstall unloads every service an install loaded.
    grep -E '^(dir|file|service) ' "$manifest" || true
  fi
  for c in "${created[@]}"; do echo "$c"; done
  if [ "$no_service" = 0 ]; then
    # service <system|user> <label_given: yes|no> [label]; the label is present only when given.
    echo "service $([ "$system" = 1 ] && echo system || echo user) $([ -n "$label" ] && echo "yes $label" || echo no)"
  fi
} > "$manifest.new"
# De-duplicate while keeping order.
awk '!seen[$0]++' "$manifest.new" > "$manifest" && rm -f "$manifest.new"
if [ -n "$run_user" ]; then
  for pth in ${new_paths[@]+"${new_paths[@]}"} "$bin" "$manifest"; do chown "$run_user" "$pth"; done
fi
[ "$existing_bin" = 1 ] && echo "install: replaced $bin"
echo "install: binary at $bin"

if [ "$no_service" = 1 ]; then
  echo "install: --no-service given; the service was not installed"
  exit 0
fi

export KEEPALIVE_PROXY_HOME=$home_dir
[ -n "$port" ] && export KEEPALIVE_PROXY_PORT=$port
if [ "$system" = 0 ] && [ "$(id -u)" = 0 ]; then
  # The user service must be loaded in the invoking user's own domain, where uninstall.sh
  # unloads it; a root run with nobody to act for would load it in root's domain.
  [ -n "$run_user" ] || die "refusing to install the per-user service as root; run as a normal user, use sudo from one, or pass --system"
  sudo -u "$run_user" -H env ${KEEPALIVE_PROXY_PORT:+KEEPALIVE_PROXY_PORT="$KEEPALIVE_PROXY_PORT"} \
    KEEPALIVE_PROXY_HOME="$home_dir" "$bin" "${service_args[@]}" \
    || die "'service install' failed; the binary stays at $bin (run scripts/uninstall.sh to remove it)"
elif ! "$bin" "${service_args[@]}"; then
  die "'service install' failed; the binary stays at $bin (run scripts/uninstall.sh to remove it)"
fi
echo "install: done"
