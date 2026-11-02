#!/usr/bin/env bash
# Builds the netwatch container image from the committed tree (ADR-0041), as
# build-rpm.sh builds the RPM: uncommitted changes are not built, so an image always
# corresponds to a commit. Tags netwatch:<version> and netwatch:latest.
#
#   ./packaging/build-container.sh           with podman, or docker if podman is missing
#   CONTAINER_ENGINE=docker ./packaging/build-container.sh
#   EBPF=0 ./packaging/build-container.sh    without drop reasons (no nightly toolchain)
#
# Extra arguments go to the engine's build command, for example --platform.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -n "$(git status --porcelain)" ]]; then
    echo "error: commit or stash your changes first; images are built from HEAD" >&2
    exit 1
fi
engine=${CONTAINER_ENGINE:-$(command -v podman || command -v docker || true)}
[[ -n "$engine" ]] || { echo "error: install podman or docker" >&2; exit 1; }

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
commit=$(git rev-parse HEAD)
context="$PWD/target/container-context"
rm -rf "$context"
mkdir -p "$context"
git archive HEAD | tar -x -C "$context"

"$engine" build \
    --file "$context/packaging/Containerfile" \
    --build-arg "EBPF=${EBPF:-1}" \
    --label "org.opencontainers.image.version=$version" \
    --label "org.opencontainers.image.revision=$commit" \
    --tag "netwatch:$version" --tag netwatch:latest \
    "$@" "$context"
echo "built netwatch:$version ($commit)"
