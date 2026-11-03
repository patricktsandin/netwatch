# Deploying netwatch

There are three ways to install (ADR-0041):

- [the RPM](#rpm), on Fedora, RHEL, and their relatives
- [the install script](#other-distros), on any other systemd distro
- [a container](#container), with podman or docker

## RPM

```sh
./packaging/build-rpm.sh                       # builds from HEAD into target/rpmbuild
sudo dnf install target/rpmbuild/RPMS/x86_64/netwatch-*.rpm
sudo systemctl enable --now netwatch
```

The package installs the binary to `/usr/bin`, the unit, the `netwatch` system user
(created by RPM from `/usr/lib/sysusers.d/netwatch.conf`), the configuration at
`/etc/netwatch/netwatch.toml` (kept across upgrades), and the state directory
`/var/lib/netwatch`. Building needs `cargo` (via rustup) and `npm` on `PATH`, plus
network access.

The UI is then at <http://127.0.0.1:8732>. ICMP echo probes use unprivileged ping
sockets, so the `netwatch` group must be within `net.ipv4.ping_group_range`. On Fedora
the default (`0 2147483647`) already includes every group.

## Other distros

`packaging/install.sh` builds from this tree and installs the same pieces as the RPM:
the binary in `/usr/local/bin`, the unit in `/etc/systemd/system`, the `netwatch`
user, and `/etc/netwatch/netwatch.toml`. It then starts the service. Run it as your
own user; it builds as you, then uses `sudo` to install.

```sh
./packaging/install.sh                 # build, install, enable, and start
./packaging/install.sh --no-start      # install only
./packaging/install.sh --prefix /opt/netwatch
./packaging/install.sh --uninstall     # keeps /etc/netwatch and /var/lib/netwatch
./packaging/install.sh --purge         # removes those and the netwatch user too
```

Building needs Rust through [rustup](https://rustup.rs) (it picks up the pinned
toolchain from `rust-toolchain.toml`), Node.js 22 with npm, a C compiler, and network
access. Drop reasons (ADR-0037) also need the nightly toolchain with `rust-src` and
[`bpf-linker`](https://github.com/aya-rs/bpf-linker/releases). Without them, the
script builds without drop reasons and says so.

Running the script again upgrades in place and restarts the service. An existing
configuration is never replaced. `--destdir DIR` stages the files in `DIR` and
changes nothing else, for packagers. The script refuses to run where the RPM is
installed.

## Container

`packaging/build-container.sh` builds the image `netwatch:<version>` from the
committed tree, with podman, or with docker if podman is missing
(`CONTAINER_ENGINE=docker` picks docker). `EBPF=0` builds without drop reasons.

```sh
./packaging/build-container.sh
sudo podman run -d --name netwatch --network host --uts host \
  --cap-drop all --cap-add NET_RAW,SYSLOG,BPF,PERFMON --device /dev/kmsg \
  -v /run/systemd/resolve:/run/systemd/resolve:ro \
  -v netwatch-config:/etc/netwatch -v netwatch-data:/var/lib/netwatch \
  netwatch:latest
```

With docker, use the same options with `docker run`. Each option matters:

| Option | Why |
|---|---|
| `--network host` | Required. netwatch measures the network namespace it runs in, which must be the host's. The UI is then on the host's `127.0.0.1:8732`. |
| `--uts host` | The report names the host by its real hostname. |
| `--cap-drop all --cap-add …` | Only the four capabilities ADR-0007 allows. Any one may be left out, and its sources are then listed as disabled. |
| `--device /dev/kmsg` | Kernel log lines as event markers (needs `SYSLOG`). |
| `-v …:/etc/netwatch` | The configuration, which the settings page rewrites. The first start copies the defaults in. |
| `-v …:/var/lib/netwatch` | The history database. |
| `-v /run/dbus/system_bus_socket:/run/dbus/system_bus_socket` | Optional: cellular stats from ModemManager. |
| `-v /run/systemd/resolve:/run/systemd/resolve:ro` | Needed on hosts that run systemd-resolved (Fedora, Ubuntu, and others). There the container's `/etc/resolv.conf` names only the local stub, so the `system` DNS target has no upstream resolver and is left out. Leave it out where `/run/systemd/resolve` doesn't exist. |

Run it rootful (`sudo podman`) for everything. A rootless container's capabilities
apply only inside its user namespace, so the kernel log and drop reasons are
disabled. ICMP, DNS, HTTPS, and hop probes still work.

`podman stop netwatch` stops it cleanly. The image sets `SIGINT` as its stop signal.
To use a different listen address or database path, set `NETWATCH_LISTEN` or
`NETWATCH_DB` with `-e`.

## Requirements on the host

ICMP probes use unprivileged ping sockets, so the `netwatch` group must be within
`net.ipv4.ping_group_range`. Fedora's default (`0 2147483647`) includes every group;
containers often default to `0 0`, which excludes it. If it's excluded, the kernel row
lists "ICMP probes: permission denied" under disabled sources. To allow it:

```sh
echo 'net.ipv4.ping_group_range = 0 2147483647' | sudo tee /etc/sysctl.d/50-netwatch.conf
sudo sysctl --system
```

## Configuration

Targets, the listen address, and the database path come from the file named by
`NETWATCH_CONFIG`, which the service's unit sets to `/etc/netwatch/netwatch.toml`.
Without it, the built-in defaults in `deploy/netwatch.toml` apply: copy that file to
start from.
Every target needs a stable `id`, because history is keyed by it.

```sh
sudo install -D -m 0644 deploy/netwatch.toml /etc/netwatch/netwatch.toml
```
