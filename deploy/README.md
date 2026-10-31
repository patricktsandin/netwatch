# Deploying netwatch

```sh
(cd web && npm ci && npm run build)          # the UI is embedded at build time
cargo build --release
sudo install -m 0755 target/release/netwatch /usr/local/bin/netwatch
sudo install -m 0644 deploy/netwatch.sysusers /usr/lib/sysusers.d/netwatch.conf
sudo systemd-sysusers
sudo install -m 0644 deploy/netwatch.service /etc/systemd/system/netwatch.service
sudo systemctl daemon-reload
sudo systemctl enable --now netwatch
```

The UI is then at <http://127.0.0.1:8732>. ICMP echo probes use unprivileged ping
sockets, so the `netwatch` group must be within `net.ipv4.ping_group_range`. On Fedora
the default (`0 2147483647`) already includes every group.

## Configuration

Targets, the listen address, and the database path come from
`/etc/netwatch/netwatch.toml` (or the file named by `NETWATCH_CONFIG`). Without one,
the built-in defaults in `deploy/netwatch.toml` apply: copy that file to start from.
Every target needs a stable `id`, because history is keyed by it.

```sh
sudo install -D -m 0644 deploy/netwatch.toml /etc/netwatch/netwatch.toml
```
