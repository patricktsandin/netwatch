#!/usr/bin/env bash
# Builds the netwatch RPM (and SRPM) from the committed tree, into target/rpmbuild.
# Uncommitted changes are not packaged; the script refuses to run with any, so a
# package always corresponds to a commit.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ -n "$(git status --porcelain)" ]]; then
    echo "error: commit or stash your changes first; packages are built from HEAD" >&2
    exit 1
fi

version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)
spec_version=$(sed -n 's/^Version: *//p' packaging/netwatch.spec)
if [[ "$version" != "$spec_version" ]]; then
    echo "error: Cargo.toml says $version but netwatch.spec says $spec_version" >&2
    exit 1
fi
top="$PWD/target/rpmbuild"
mkdir -p "$top"/{SOURCES,SPECS,BUILD,RPMS,SRPMS}
git archive --format=tar.gz --prefix="netwatch-$version/" -o "$top/SOURCES/netwatch-$version.tar.gz" HEAD

rpmbuild -ba packaging/netwatch.spec --define "_topdir $top"

find "$top/RPMS" "$top/SRPMS" -name '*.rpm' -newer "$top/SOURCES/netwatch-$version.tar.gz"
