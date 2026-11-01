# Deploying netwatch

The supported way to install is the RPM:

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

Targets, the listen address, and the database path come from
`/etc/netwatch/netwatch.toml` (or the file named by `NETWATCH_CONFIG`). Without one,
the built-in defaults in `deploy/netwatch.toml` apply: copy that file to start from.
Every target needs a stable `id`, because history is keyed by it.

```sh
sudo install -D -m 0644 deploy/netwatch.toml /etc/netwatch/netwatch.toml
```
