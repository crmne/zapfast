---
title: Download
description: Get ZapFast for Linux, macOS, or Windows, with install instructions for each.
nav_order: 1
---

{% assign v = site.zapfast_version %}
{% assign name = site.release_asset_prefix %}
{% assign app = site.release_app_name %}
{% assign base = "https://github.com/crmne/zapfast/releases/download/v" | append: v %}

ZapFast was previously called FastsApp. Version 0.13.0 introduces the new
package and executable names. Your existing session and local data move
automatically when you first start ZapFast; quit FastsApp before upgrading.

The current version is **v{{ v }}**. SHA-256 checksums are in
[checksums.txt]({{ base }}/checksums.txt). Older versions are on the
[releases page](https://github.com/crmne/zapfast/releases).

## Linux

On Arch Linux and derivatives, install from the
[AUR](https://aur.archlinux.org/packages/{{ name }}-bin):

```sh
yay -S {{ name }}-bin    # prebuilt
yay -S {{ name }}        # builds from the release source
yay -S {{ name }}-git    # builds from the latest commit
```

With [Nix](https://nixos.org), install from the flake:

```sh
nix profile install github:crmne/zapfast
```

NixOS configurations can add the repository as a flake input and include
`inputs.zapfast.packages.${pkgs.system}.default` in
`environment.systemPackages`.

With [Home Manager](https://github.com/nix-community/home-manager), the
flake's module installs ZapFast and declares its settings:

```nix
imports = [ inputs.zapfast.homeManagerModules.default ];

programs.zapfast = {
  enable = true;
  settings = {
    check_for_updates = false;
    message_sound = "none";
  };
};
```

`settings` takes the keys of `settings.json` (see
[Settings & Files](/settings-and-files/)). They are merged into the file on
every activation, so the window size, recent emoji, and lock verifiers the app
keeps there are left alone. A declared key that is changed in the app is reset
on the next activation. Per-account preferences such as read receipts and
notifications live in each account's own file and cannot be declared here.
Restart ZapFast after activating: it reads the file at
start, and its next save writes back the values it already had.

Values in `settings` are copied to the Nix store, which every user on the
computer can read, so keep secrets such as proxy credentials or a GIPHY key
out of it. Either set them in the app and leave them undeclared, or name a
file for each key in `secretFiles`, for example one that sops-nix or agenix
decrypts. The module reads those files when it activates:

```nix
programs.zapfast.secretFiles.proxy = config.sops.secrets.zapfast-proxy.path;
```

For other distributions, download a tarball with the binary, desktop file,
and icon:

- [{{ name }}-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz]({{ base }}/{{ name }}-v{{ v }}-x86_64-unknown-linux-gnu.tar.gz)
- [{{ name }}-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz]({{ base }}/{{ name }}-v{{ v }}-aarch64-unknown-linux-gnu.tar.gz)

Or take the AppImage: one file to make executable and run, with no
installation. It uses the same libraries as the tarball and needs FUSE (or
run it with `--appimage-extract-and-run`). It does not update itself; download
the new file when {{ app }} says a release is out.

- [{{ name }}-{{ v }}-x86_64.AppImage]({{ base }}/{{ name }}-{{ v }}-x86_64.AppImage)
- [{{ name }}-{{ v }}-aarch64.AppImage]({{ base }}/{{ name }}-{{ v }}-aarch64.AppImage)

{{ app }} needs glibc 2.36 or newer (Debian 12, Ubuntu 22.10, Fedora 37 and
newer), the standard egui libraries, ALSA and libpipewire 0.3.53 or newer:
`libglvnd`, `libxkbcommon`, `wayland`, `libx11`, `alsa-lib` and `libpipewire` (on
Debian or Ubuntu: `libasound2t64`, `libpipewire-0.3-0t64`, `libgl1`,
`libxkbcommon0`, `libwayland-client0`). Sound goes to PipeWire itself, else to
PulseAudio, else to ALSA.
Emoji come from the desktop's colour emoji font when one is installed
(`noto-fonts-emoji`, `fonts-noto-color-emoji` on Debian), else from the copy
{{ app }} bundles. The file picker uses `xdg-desktop-portal`.

A Flatpak bundle is being prepared; {{ app }} is not on Flathub yet.

## macOS

One download for both Apple Silicon and Intel:

- [{{ name }}-v{{ v }}-macos-universal.dmg]({{ base }}/{{ name }}-v{{ v }}-macos-universal.dmg)

Open it and drag **{{ app }}** to Applications.

If you use [Homebrew](https://brew.sh), you can install it with:

```sh
brew install --cask crmne/tap/zapfast
```

The app is signed with Developer ID and notarized by Apple. The DMG includes
a validated notarization ticket.

When upgrading from FastsApp, remove the old FastsApp application bundle
after installing ZapFast.

## Windows

The installer is a Windows Installer package (`.msi`). It adds {{ app }} to
the Start menu and needs no administrator rights. Choose x86_64 for most PCs
or aarch64 for Windows on ARM:

- [{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.msi]({{ base }}/{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.msi)
- [{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.msi]({{ base }}/{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.msi)

If you installed {{ app }} with the older setup program (`-setup.exe`), 0.20.0
and later move to the package through their own updates. On 0.19.0 or earlier,
run the `.msi` once: it installs into the same folder and replaces the old
installer's entry in Settings > Apps. Your session, chats and settings stay.

To run {{ app }} without installing it, download a zip, extract it, and run
`{{ name }}.exe`.

- [{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.zip]({{ base }}/{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.zip]({{ base }}/{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.zip)

{{ app }} draws with OpenGL and needs a graphics driver with OpenGL 2.1 or
newer. Install the driver from the maker of your graphics chip: the Microsoft
Basic Display Adapter, some virtual machines, and some remote desktop sessions
offer no usable OpenGL, and Windows then shows a message instead of starting.

SmartScreen may warn about an unknown publisher on first run. Choose **More
info**, then **Run anyway**.
