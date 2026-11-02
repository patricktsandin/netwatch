#!/bin/sh
# The container's entrypoint (ADR-0041): seeds an empty configuration volume with the
# default configuration, so that the settings page has a file to rewrite (ADR-0035),
# then becomes netwatch. A configuration already there is never replaced.
set -eu
if [ ! -e /etc/netwatch/netwatch.toml ]; then
    cp /usr/share/netwatch/netwatch.toml /etc/netwatch/netwatch.toml
fi
exec /usr/bin/netwatch "$@"
