#!/usr/bin/env bash
# Installs netwatch from this source tree on any systemd distro, the same pieces the
# RPM installs (ADR-0041). Builds as the invoking user, then uses sudo for the rest.
#
#   ./packaging/install.sh                 build, install under /usr/local, start
#   ./packaging/install.sh --no-build      install target/release/netwatch as built
#   ./packaging/install.sh --no-start      install, but don't enable or start
#   ./packaging/install.sh --destdir DIR   stage the files in DIR; touch nothing else
#   ./packaging/install.sh --uninstall     remove it; keep configuration and history
#   ./packaging/install.sh --purge         remove it with configuration and history
#
# --prefix DIR picks where the binary goes (default /usr/local). The configuration
# stays in /etc/netwatch and the history in /var/lib/netwatch, as with the RPM.
set -euo pipefail
cd "$(dirname "$0")/.."

prefix=/usr/local
destdir=
build=1
start=1
action=install
while (($#)); do
    case "$1" in
        --prefix) prefix=${2:?--prefix needs a directory}; shift ;;
        --destdir) destdir=${2:?--destdir needs a directory}; shift ;;
        --no-build) build=0 ;;
        --no-start) start=0 ;;
        --uninstall) action=uninstall ;;
        --purge) action=purge ;;
        -h | --help) sed -n '2,14s/^# \{0,1\}//p' "$0"; exit 0 ;;
        *) echo "error: unknown option $1 (see --help)" >&2; exit 2 ;;
    esac
    shift
done

bin="$prefix/bin/netwatch"
unit=/etc/systemd/system/netwatch.service
sysusers=/etc/sysusers.d/netwatch.conf
docs="$prefix/share/doc/netwatch"
config_dir=/etc/netwatch
state_dir=/var/lib/netwatch

# Everything that changes the system goes through `as_root`. A staged install
# writes only below $destdir, as the invoking user.
if [[ -n "$destdir" || $EUID -eq 0 ]]; then
    as_root() { "$@"; }
else
    command -v sudo >/dev/null || { echo "error: run as root, or install sudo" >&2; exit 1; }
    as_root() { sudo "$@"; }
fi
have() { command -v "$1" >/dev/null; }
say() { printf '==> %s\n' "$*"; }

if [[ -z "$destdir" ]]; then
    have systemctl || { echo "error: netwatch runs as a systemd service; systemctl is missing" >&2; exit 1; }
    if have rpm && rpm -q netwatch >/dev/null 2>&1; then
        echo "error: the netwatch RPM is installed; use dnf to upgrade or remove it" >&2
        exit 1
    fi
fi

uninstall() {
    if systemctl list-unit-files netwatch.service >/dev/null 2>&1; then
        say "stopping netwatch"
        as_root systemctl disable --now netwatch.service || true
    fi
    say "removing $bin, $docs, $unit, and $sysusers"
    as_root rm -f "$bin" "$unit" "$sysusers"
    as_root rm -rf "$docs"
    as_root systemctl daemon-reload
    if [[ "$action" == purge ]]; then
        say "removing $config_dir, $state_dir, and the netwatch user"
        as_root rm -rf "$config_dir" "$state_dir"
        if getent passwd netwatch >/dev/null; then as_root userdel netwatch || true; fi
        if getent group netwatch >/dev/null; then as_root groupdel netwatch || true; fi
    else
        say "kept $config_dir, $state_dir, and the netwatch user (--purge removes them)"
    fi
}

if [[ "$action" != install ]]; then
    [[ -z "$destdir" ]] || { echo "error: --destdir only stages an install" >&2; exit 2; }
    uninstall
    exit 0
fi

if ((build)); then
    if [[ $EUID -eq 0 ]]; then
        echo "error: build as your own user, not root; sudo is used only to install" >&2
        echo "       (or build first, then run this with --no-build)" >&2
        exit 1
    fi
    have cargo || { echo "error: cargo is missing; install Rust with rustup (https://rustup.rs)" >&2; exit 1; }
    have npm || { echo "error: npm is missing; install Node.js 22" >&2; exit 1; }
    # The eBPF program needs nightly with rust-src and bpf-linker (ADR-0008). Without
    # them netwatch still builds and reports drop reasons as disabled.
    if [[ -z "${AYA_BUILD_SKIP:-}" ]] && ! { have bpf-linker && have rustup && rustup run nightly rustc --version >/dev/null 2>&1; }; then
        say "nightly Rust or bpf-linker is missing: building without drop reasons (ADR-0037)"
        export AYA_BUILD_SKIP=1
    fi
    say "building the web UI"
    (cd web && npm ci --no-audit --no-fund && npm run build)
    say "building netwatch"
    cargo build --release --locked -p netwatch
fi
[[ -x target/release/netwatch ]] || { echo "error: target/release/netwatch isn't built" >&2; exit 1; }

say "installing into ${destdir:-/}"
as_root install -D -s -m 0755 target/release/netwatch "$destdir$bin"
as_root install -d "$destdir$docs"
as_root install -m 0644 README.md "$destdir$docs/README.md"
as_root install -m 0644 deploy/README.md "$destdir$docs/DEPLOYING.md"
as_root cp -r docs/adr docs/glossary.md docs/context-map.md "$destdir$docs/"
# The unit is the RPM's, pointed at this binary and its documentation.
sed -e "s|^ExecStart=/usr/bin/netwatch|ExecStart=$bin|" \
    -e "s|^Documentation=.*|Documentation=file://$docs/README.md|" \
    -e 's|^# Installed by the netwatch RPM.*|# Installed by packaging/install.sh (ADR-0041). Settings and targets live in|' \
    deploy/netwatch.service | as_root install -D -m 0644 /dev/stdin "$destdir$unit"
as_root install -D -m 0644 deploy/netwatch.sysusers "$destdir$sysusers"

if [[ -n "$destdir" ]]; then
    install -d "$destdir$config_dir"
    sed "s|^database = .*|database = \"$state_dir/netwatch.db\"|" deploy/netwatch.toml >"$destdir$config_dir/netwatch.toml"
    say "staged; nothing outside $destdir was changed"
    exit 0
fi

if ! getent passwd netwatch >/dev/null; then
    say "creating the netwatch user"
    if have systemd-sysusers; then
        as_root systemd-sysusers "$sysusers"
    else
        nologin=$(command -v nologin || echo /sbin/nologin)
        as_root useradd --system --user-group --home-dir "$state_dir" --no-create-home \
            --shell "$nologin" --comment "netwatch network monitor" netwatch
    fi
fi

# The settings page rewrites the file through a temporary file beside it (ADR-0035),
# so the service's group may write both. An existing configuration is never replaced.
as_root install -d -o root -g netwatch -m 0775 "$config_dir"
if as_root test -e "$config_dir/netwatch.toml"; then
    say "kept the existing $config_dir/netwatch.toml"
else
    sed "s|^database = .*|database = \"$state_dir/netwatch.db\"|" deploy/netwatch.toml |
        as_root install -o root -g netwatch -m 0664 /dev/stdin "$config_dir/netwatch.toml"
fi

as_root systemctl daemon-reload
if ((start)); then
    if systemctl is-active --quiet netwatch.service; then
        say "restarting netwatch"
        as_root systemctl restart netwatch.service
    else
        say "starting netwatch"
        as_root systemctl enable --now netwatch.service
    fi
fi

# ICMP probes use unprivileged ping sockets (deploy/README.md).
range=$(sysctl -n net.ipv4.ping_group_range 2>/dev/null | tr -s '[:space:]' ' ' || echo "0 0")
gid=$(getent group netwatch | cut -d: -f3)
read -r low high <<<"$range"
if ((gid < low || gid > high)); then
    echo "warning: net.ipv4.ping_group_range ($range) excludes the netwatch group ($gid)," >&2
    echo "         so ICMP probes are disabled. deploy/README.md says how to allow them." >&2
fi
say "done: the UI is at http://127.0.0.1:8732"
