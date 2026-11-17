# ZapFast built from its release source, for Fedora COPR (paolino/zapfast) and
# openSUSE OBS. Cargo fetches the locked dependencies while building, so the
# COPR project and the OBS package need network access during builds.

Name:           zapfast
Version:        @VERSION@
Release:        1%{?dist}
Summary:        Fast native WhatsApp client
License:        MIT
URL:            https://zapfast.rocks
Source0:        https://github.com/crmne/zapfast/archive/refs/tags/v%{version}.tar.gz#/%{name}-%{version}.tar.gz

# ring compiles its own C, and LTO objects in its archive leave the link
# without ring_core_* symbols, as on Arch.
%global _lto_cflags %{nil}
# The binary is stripped of its source paths below; debuginfo would only
# carry them back.
%global debug_package %{nil}

ExclusiveArch:  x86_64 aarch64
BuildRequires:  rust >= 1.98
BuildRequires:  cargo
BuildRequires:  cmake
BuildRequires:  clang
BuildRequires:  clang-devel
BuildRequires:  gcc-c++
BuildRequires:  perl
BuildRequires:  pkgconfig(alsa)
BuildRequires:  pkgconfig(libpipewire-0.3)
# ZapFast opens these libraries while running (OpenGL, the window system and
# the keyboard), so rpmbuild cannot see them. They are required by library
# name, which Fedora and openSUSE share, not by package name, which they do
# not (libX11 and libX11-6). rpmbuild adds the linked ones (sound, the C++
# runtime) itself. Both architectures are 64-bit.
Requires:       libGL.so.1()(64bit)
Requires:       libEGL.so.1()(64bit)
Requires:       libxkbcommon.so.0()(64bit)
Requires:       libxkbcommon-x11.so.0()(64bit)
Requires:       libwayland-client.so.0()(64bit)
Requires:       libwayland-cursor.so.0()(64bit)
Requires:       libwayland-egl.so.1()(64bit)
Requires:       libX11.so.6()(64bit)
Requires:       libX11-xcb.so.1()(64bit)
Requires:       libXcursor.so.1()(64bit)
Requires:       libXi.so.6()(64bit)
Requires:       libXrandr.so.2()(64bit)
Recommends:     google-noto-color-emoji-fonts
Recommends:     xdg-desktop-portal
Provides:       fastsapp = %{version}-%{release}
Obsoletes:      fastsapp < %{version}-%{release}

%description
ZapFast is a native WhatsApp client for the desktop, written in Rust with
egui: no browser engine, about 200 MB of memory when idle. It links as a
companion device of your phone and keeps your chats in an encrypted local
archive.

%prep
%autosetup -n %{name}-%{version}

%build
export CARGO_TARGET_DIR=target
export RUSTFLAGS="${RUSTFLAGS:-} --remap-path-prefix=%{_builddir}=/"
cargo build --locked --release

%install
install -Dm755 target/release/zapfast %{buildroot}%{_bindir}/zapfast
install -Dm644 packaging/applications/zapfast.desktop %{buildroot}%{_datadir}/applications/zapfast.desktop
install -Dm644 packaging/icons/zapfast.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/zapfast.svg
install -Dm644 contrib/omarchy/zapfast.json.tpl %{buildroot}%{_datadir}/zapfast/omarchy/zapfast.json.tpl
install -Dm755 contrib/omarchy/zapfast-theme %{buildroot}%{_datadir}/zapfast/omarchy/zapfast-theme

%files
%license LICENSE THIRD-PARTY-NOTICES.md
%doc README.md
%{_bindir}/zapfast
%{_datadir}/applications/zapfast.desktop
%{_datadir}/icons/hicolor/scalable/apps/zapfast.svg
%{_datadir}/zapfast/

%changelog
* @DATE@ Carmine Paolino <carmine@paolino.me> - @VERSION@-1
- ZapFast @VERSION@: https://github.com/crmne/zapfast/releases/tag/v@VERSION@
