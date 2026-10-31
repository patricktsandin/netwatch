# netwatch: a continuous network monitor with a web UI.
#
# Built by packaging/build-rpm.sh, which makes the source tarball from the committed
# tree and runs rpmbuild. %build needs the project's toolchains on PATH (rustup's
# cargo, which honors rust-toolchain.toml, and npm) and network access for crates and
# npm packages, so this builds on a developer machine, not in an offline mock chroot.

Name:           netwatch
Version:        %{netwatch_version}
Release:        1%{?dist}
Summary:        Continuous network monitor in the spirit of mtr, with a web UI
# No license has been chosen for netwatch yet. Replace this before distributing.
License:        LicenseRef-Not-Yet-Chosen
URL:            https://github.com/example/netwatch
Source0:        %{name}-%{version}.tar.gz

ExclusiveArch:  x86_64 aarch64
BuildRequires:  systemd-rpm-macros
%{?systemd_requires}

# The binary carries the whole UI embedded; there is nothing to split out.
%global debug_package %{nil}

%description
netwatch watches this host's path to the network layer by layer (kernel, NIC,
local interface, hops 1-3, and ICMP, DNS, and HTTPS targets), follows every
change in routing and interfaces, and graphs it live and historically in a web UI
served on localhost.

%prep
%autosetup -n %{name}-%{version}

%build
(cd web && npm ci --no-audit --no-fund && npm run build)
cargo build --release --locked -p netwatch

%install
# deploy/README.md would collide with README.md among the docs.
cp deploy/README.md DEPLOYING.md
install -D -m 0755 target/release/netwatch %{buildroot}%{_bindir}/netwatch
install -D -m 0644 deploy/netwatch.service %{buildroot}%{_unitdir}/netwatch.service
install -D -m 0644 deploy/netwatch.sysusers %{buildroot}%{_sysusersdir}/netwatch.conf
# The packaged configuration keeps its database in the service's state directory.
install -d %{buildroot}%{_sysconfdir}/netwatch
sed 's|^database = .*|database = "%{_sharedstatedir}/netwatch/netwatch.db"|' \
    deploy/netwatch.toml > %{buildroot}%{_sysconfdir}/netwatch/netwatch.toml
install -d -m 0750 %{buildroot}%{_sharedstatedir}/netwatch

%check
grep -q '^database = "%{_sharedstatedir}/netwatch/netwatch.db"$' \
    %{buildroot}%{_sysconfdir}/netwatch/netwatch.toml

%post
%systemd_post netwatch.service

%preun
%systemd_preun netwatch.service

%postun
%systemd_postun_with_restart netwatch.service

%files
%doc README.md DEPLOYING.md docs/adr docs/glossary.md docs/context-map.md
%{_bindir}/netwatch
%{_unitdir}/netwatch.service
%{_sysusersdir}/netwatch.conf
%dir %{_sysconfdir}/netwatch
%config(noreplace) %{_sysconfdir}/netwatch/netwatch.toml
%attr(0750, netwatch, netwatch) %dir %{_sharedstatedir}/netwatch

%changelog
* Sat Oct 31 2026 Patrick <patrick@sandin.tech> - 0.1.0-1
- First package.
