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

{{ app }} needs the standard egui libraries and ALSA:
`libglvnd`, `libxkbcommon`, `wayland`, `libx11`, and `alsa-lib` (on
Debian or Ubuntu: `libasound2`, `libgl1`, `libxkbcommon0`, `libwayland-client0`).
Emoji come from the desktop's colour emoji font when one is installed
(`noto-fonts-emoji`, `fonts-noto-color-emoji` on Debian), else from the copy
{{ app }} bundles. The file picker uses `xdg-desktop-portal`.

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

The installer adds {{ app }} to the Start menu and needs no administrator
rights. Choose x86_64 for most PCs or aarch64 for Windows on ARM:

- [{{ name }}-v{{ v }}-x86_64-pc-windows-msvc-setup.exe]({{ base }}/{{ name }}-v{{ v }}-x86_64-pc-windows-msvc-setup.exe)
- [{{ name }}-v{{ v }}-aarch64-pc-windows-msvc-setup.exe]({{ base }}/{{ name }}-v{{ v }}-aarch64-pc-windows-msvc-setup.exe)

To run {{ app }} without installing it, download a zip, extract it, and run
`{{ name }}.exe`.

- [{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.zip]({{ base }}/{{ name }}-v{{ v }}-x86_64-pc-windows-msvc.zip)
- [{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.zip]({{ base }}/{{ name }}-v{{ v }}-aarch64-pc-windows-msvc.zip)

SmartScreen may warn about an unknown publisher on first run. Choose **More
info**, then **Run anyway**.
