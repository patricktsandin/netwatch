# 41. Installing without RPM, and running in a container

Date: 2026-11-02

## Status

Accepted. Adds two ways to install alongside the RPM (`packaging/netwatch.spec`).

## Context

The RPM serves Fedora and its relatives. People on Debian, Arch, and other distros
need another way to install. Others want netwatch in a container, where their
existing tooling manages it.

netwatch measures the host's own network (ADR-0002, ADR-0003). It reads the host's
rtnetlink, `/proc/net`, its neighbor tables, `/dev/kmsg`, and its drop reasons. A
container in its own network namespace would measure the container's virtual
network instead. Every capability netwatch uses is optional (ADR-0007), so a
container that lacks one still starts and lists the missing source as disabled.

## Decision

- **`packaging/install.sh` installs from source on any systemd distro.** It builds as
  the invoking user, then uses `sudo` for the rest. It installs the same pieces as the
  RPM, under `/usr/local` by default:
  - the binary
  - the unit, as `/etc/systemd/system/netwatch.service`, pointed at that binary
  - the `netwatch` user, through `systemd-sysusers` (or `useradd` where that is
    missing)
  - `/etc/netwatch/netwatch.toml`, owned and permissioned as the RPM sets it
    (ADR-0035)

  The script never replaces an existing configuration. `--uninstall` removes what
  the script installed and keeps the configuration and history; `--purge` removes
  those too. `--destdir` stages the files without touching the system, for testing
  and for other packagers.
- **`packaging/build-container.sh` builds an OCI image** with podman (or docker) from
  `packaging/Containerfile`, using the committed tree as the RPM does. The build
  stage installs the same toolchains as the dev host, eBPF included. `EBPF=0` builds
  without eBPF (`AYA_BUILD_SKIP=1`). The runtime stage is a minimal image of the same
  distribution, so the binary's glibc matches.
- **The container runs in the host's network namespace** (`--network host`), so it
  measures the host's paths. It also runs in the host's UTS namespace, so the report
  names the real host. The UI then listens on the host's loopback, as it does without
  a container (ADR-0010).
- **Inside the container netwatch runs as root, holding only ADR-0007's four
  capabilities.** The run command drops every capability and adds back `NET_RAW`,
  `SYSLOG`, `BPF`, and `PERFMON`. Container engines don't grant added capabilities to
  a non-root user as ambient capabilities. File capabilities would refuse to execute
  wherever a capability is missing, which breaks "never fail to start" (ADR-0007).
- **The configuration and the database live in two volumes**, `/etc/netwatch` and
  `/var/lib/netwatch`. On the first start the entrypoint copies the image's default
  configuration into an empty `/etc/netwatch`. The settings page can then rewrite it
  (ADR-0035).

## Consequences

- A rootless container's capabilities stop at its user namespace. ARP/NDP probes,
  `/dev/kmsg`, and drop reasons are then disabled. ICMP, DNS, HTTPS, and the hop
  probes still work, provided the host's `ping_group_range` allows them. A rootful
  container gets everything.
- On a host that runs systemd-resolved, the container's `/etc/resolv.conf` names only
  the loopback stub, which is not an upstream resolver (ADR-0027). The run command
  mounts `/run/systemd/resolve` read-only, so netwatch finds the real one.
- Wi-Fi works over netlink. Cellular needs the host's D-Bus system socket mounted
  into the container (`/run/dbus/system_bus_socket`).
- The install script installs files the package manager doesn't know about.
  `--uninstall` is the way to remove them. The script refuses to run where the RPM is
  installed, because two units and two binaries would conflict.
- The nightly toolchain the container build uses for eBPF is not pinned, which is
  also true of the dev host (ADR-0008).
