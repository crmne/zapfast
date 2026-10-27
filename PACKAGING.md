# Packaging

[`native-packages.yaml`](native-packages.yaml) is the packaging configuration:
it pins the shared CLI and nFPM versions and declares Linux amd64/arm64 inputs,
DEB/RPM/AppImage contents, dependencies, recipe templates and downstream repositories.
Application assets and native recipes stay in `packaging/`.

Version 0.13.0 introduces the ZapFast name and `zapfast` binary. Its AUR recipes
provide and replace the corresponding FastsApp packages. The GitHub repository is
`crmne/zapfast`, so source archives extract into `zapfast-VERSION`. Use the
configuration from the matching tag to rebuild an older FastsApp release.
Existing release files keep their original names.

```sh
gem install native-packages --version 0.8.1
native-packages validate
native-packages doctor --target linux-amd64 --target linux-arm64
native-packages build --release v1.2.3 --target linux-amd64 --target linux-arm64
```

Replace `v1.2.3` with an existing stable application release. Local use also
requires nFPM 2.47.0, `bsdtar`, `readelf` and, for the AppImage,
`mksquashfs` (squashfs-tools); AUR generation needs `makepkg`
or Docker. CI installs its tooling. To package local release archives, put
every configured input and recipe asset under `dist/`, then run
`native-packages build --version 1.2.3 --target linux-amd64 --target linux-arm64`. Outputs go to
`dist/packages/1.2.3`; use `--output` for a fresh destination when rebuilding.

Stable tags run the existing native build jobs first. After binaries and
`checksums.txt` are published, the shared workflow verifies their hashes,
builds the configured packages, and attaches them to the GitHub release.
Configured recipes are attached as an archive. Package checksums are separate
from the original binary checksums. PR validation never publishes.

Review or publish an existing build with the same installed CLI:

```sh
native-packages publish --from dist/packages/1.2.3 --to github
native-packages repositories
native-packages status --offline
```

For applications with configured AUR or Homebrew destinations, stage the
recipes with `native-packages stage TARGET dist/packages/1.2.3/recipes`,
inspect `native-packages diff TARGET`, run native package validation, and
publish with `native-packages publish TARGET`. These destinations use ignored
managed Git clones, recorded in this application's YAML configuration.
AUR automation needs `PUBLISH_AUR=true`, `AUR_SSH_KEY` and `AUR_KNOWN_HOSTS`;
Homebrew automation needs `PUBLISH_HOMEBREW=true` and
`HOMEBREW_TAP_GITHUB_TOKEN`. Enable only configured destinations.

The native macOS configuration, Windows and Flatpak build steps remain responsible
for their native artifacts. Additional nFPM formats require suitable platform
inputs and dependencies; adding a format does not port the application.
See the [shared CLI documentation](https://github.com/crmne/native-packages/tree/v0.8.1)
for commands and supported formats.

To upgrade the tool, change `tool.version` in `native-packages.yaml`, the matching immutable workflow reference, and any release-job gem installation
pin together. Applications need no packaging Gemfile, lockfile or Ruby wrapper.

## Automatic macOS notarization

`packaging/macos/entitlements.plist` grants microphone access under the hardened
runtime, and `Info.plist` supplies the permission prompt. `bundle.sh` embeds the
entitlement in its initial signature so native-packages preserves it when signing
with Developer ID. After notarization, `verify.sh` mounts the final DMG and checks
its ticket, Gatekeeper acceptance, both architectures, and microphone metadata.

The macOS release job builds the app first, then uses
`native-packages.yaml` and `packaging/macos/dmg.rb` to package it.
The shared gem signs its owned input copy, notarizes the DMG, staples and validates
Apple's ticket, and only then records final checksums. Configure these repository
secrets, which the job exposes as environment variables:

- `APPLE_CERTIFICATE_P12`: base64 PKCS#12 Developer ID Application certificate and private key.
- `APPLE_CERTIFICATE_PASSWORD`: the export password.
- `APPLE_SIGNING_IDENTITY`: exact `Developer ID Application: Name (TEAMID)` identity.
- `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD`: Apple email, Team ID and app-specific password.

A complete set enables notarization automatically. An incomplete set fails;
no values retain local builds without Developer ID signing. Application inputs
and the user's normal keychains remain unchanged. See the shared
[Apple setup and phase contract](https://github.com/crmne/native-packages/blob/v0.8.1/docs/apple-notarization.md).

After preparing `dist/macos-input` on a Mac, test packaging without publishing:

```sh
native-packages build \
  --version 1.2.3 --target macos-universal --defer-recipes --output dist/macos-packages-test
```

Secret configuration applies to future builds. Existing published DMGs retain
their original signatures; this setup does not replace release assets.

Linux releases build in a Debian 12 container, so the binaries need glibc 2.36
or newer and the packages install on Debian 12, Ubuntu 22.10 and Fedora 37
onwards. Ubuntu 22.04 is not supported: its libpipewire is older than the
0.3.53 fastframe-audio's PipeWire output builds against. native-packages derives the `libc6`/`glibc` floor from the binary; do
not declare one. DEB/RPM recipes declare runtime-loaded Wayland, X11 and EGL
libraries as well as ALSA (`libasound2t64 | libasound2`, the package's name
before and after the 64-bit time transition) and libpipewire
(`libpipewire-0.3-0t64 | libpipewire-0.3-0`, `pipewire-libs`). Sound goes to
PipeWire or PulseAudio directly, so ALSA's PulseAudio plugin is not needed.
Packaging CI builds both architectures from the published v0.13.1 fixture on
PRs; release runs use their own tag. Clean Ubuntu, Debian and Fedora containers
install and remove each package, check GUI libraries loaded with `dlopen`, and
verify desktop and theme assets. Run the same check locally with
`bash packaging/test-install.sh ubuntu:24.04 /path/to/native-packages-output`. The macOS job selects `macos-universal`
from the same configuration with `--defer-recipes`, leaving Linux inputs and AUR
recipe generation to the Linux packaging job after release assets exist.

## Windows installer

Windows releases ship the portable zip and a per-user Windows Installer
package, `zapfast-vX.Y.Z-<target>.msi`, built by native-packages from
`packaging/windows/zapfast.wxs` (`packaging/windows/msi.rb`). Releases up to
0.20.1 also shipped an Inno Setup program (`-setup.exe`); it is gone, because
the package is the one winget's manifests install, and the one signed and tested.

The package takes over a copy the Inno Setup program installed: it installs
into that folder, removes Inno's uninstaller and its Settings > Apps entry, and
leaves the user's data (session, archive, settings) alone, as they live
outside the program folder. ZapFast 0.20.0 and later update to the package by
themselves (the updater prefers the `.msi`). Older versions look for a
`-setup.exe` that new releases no longer have, so they stop finding updates;
their users download the `.msi` once (`docs/_guide/download.md` says so).
`packaging/windows/zapfast.iss` stays only so `test-installer.ps1` can make an
Inno install for the package to take over.

## Windows signing

Tag releases sign `zapfast.exe` and the MSI with Azure Artifact Signing, as
**PlentyLabs UG (haftungsbeschränkt) & Co. KG**, through native-packages:
`windows.signing` in `native-packages.yaml` turns it on,
`native-packages sign-windows` signs `zapfast.exe` before the zip and the MSI
pack it, and `native-packages build` signs the MSI before recording its
checksum. Each
signature is SHA-256 Authenticode with Microsoft's RFC 3161 timestamp, checked
to be `Valid` and PlentyLabs' before anything is attested or uploaded (see
native-packages'
[Windows code signing](https://github.com/crmne/native-packages/blob/v0.11.0/docs/_guides/windows-signing.md)).
The account, `plentylabs` in West Europe,
is shared by Carmine's Windows apps and managed in `crmne/terraform`
(`azure/signing.tf`, its README's "Code signing").

No secret is involved. The Windows build jobs run in this repository's
`windows-signing` environment, which deploys only from `v*` tags, and sign in
through GitHub's OIDC token as the Entra app `GitHub zapfast Windows signing`.
That app trusts only `repo:crmne@106617/zapfast@1350713137:environment:windows-signing` (GitHub's immutable subject, with the owner's and repository's ids) and holds
the "Artifact Signing Certificate Profile Signer" role on the account. The
environment's variables, none of them secret, are `AZURE_CLIENT_ID`,
`AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`, `AZURE_SIGNING_ENDPOINT`,
`AZURE_SIGNING_ACCOUNT` and `AZURE_SIGNING_PROFILE`.

`AZURE_SIGNING_PROFILE` names the Public Trust certificate profile
`plentylabs`, created once PlentyLabs' identity validation completed
(2026-10-26), so every tag release signs. Without the environment's signing
variables releases build unsigned; with only some of them, the Windows build
fails rather than ship unsigned.

A signature doesn't silence SmartScreen at once: reputation builds as people
download releases signed by the same publisher, so keep signing every release
with this identity.

## Flatpak

`packaging/flatpak/rocks.zapfast.ZapFast.yml` builds from source, with offline Cargo
sources generated from the selected revision's lockfile. The adjacent bundle
manifest reuses the Linux release binary, as in Spotifast. Both grant Wayland/X11,
GPU, audio, network, keyring and tray access; attachments chosen by the user use
portals. No home-directory permission is granted. `--persist=.local/state` keeps
the archive and session on Flatpak versions without `XDG_STATE_HOME`.

Generate a pinned Flathub checkout (Python needs `aiohttp`, `tomlkit` and `PyYAML`):

```sh
packaging/flatpak/flathub.sh vX.Y.Z /path/to/flathub-checkout
flatpak-builder --user --install --force-clean build-dir /path/to/flathub-checkout/rocks.zapfast.ZapFast.yml
```

Flathub submission/review is a separate publication step; the manifest alone does
not make ZapFast available in Flathub. A maintainer must submit it manually:
[Flathub's requirements](https://docs.flathub.org/docs/for-app-authors/requirements#generative-ai-policy)
prohibit AI agents from submitting or writing submission interactions and require
disclosure of generated material. Review the manifests and these changes before
submitting. The manifests use the current Freedesktop 26.08 runtime; the CI builder
container is 25.08 and installs the runtime and SDK named by the manifest. The GitHub release job includes the bundle
in `checksums.txt`. No existing release files are replaced by this change.

## winget

`packaging/winget/` holds the manifests for the `crmne.ZapFast` package in
[microsoft/winget-pkgs](https://github.com/microsoft/winget-pkgs). They install
the per-user MSI of each stable release, which also takes over a copy installed
by the Inno Setup installer; its fixed UpgradeCode tells winget which installed
program is ZapFast. v0.19.0 and earlier ship no MSI, so the first winget
version is the first release that does.

Submission is a manual step for the maintainer, after the release's assets and
`checksums.txt` exist. Nothing in CI submits it. Write the manifests from the
published release, then validate and submit them from a fork of winget-pkgs:

```sh
python3 packaging/winget/manifests.py vX.Y.Z /path/to/winget-pkgs
winget validate --manifest /path/to/winget-pkgs/manifests/c/crmne/ZapFast/X.Y.Z
winget install --manifest /path/to/winget-pkgs/manifests/c/crmne/ZapFast/X.Y.Z
```

Commit only that version's directory and open a pull request to
`microsoft/winget-pkgs` (or use `wingetcreate submit` on the directory). Its
validation installs the package and records anything else it needs. Later
releases can reuse the same command or `wingetcreate update crmne.ZapFast`;
change the templates here when the description or installers change, so the
next submission keeps them.
