---
title: Getting Started
description: Install ZapFast, link your phone, and load chat history.
nav_order: 2
---

## Install

The [Download page](/download/) has packages and archives for Linux,
macOS, and Windows.

Or build from source with a recent stable [Rust](https://rustup.rs):

```sh
git clone https://github.com/crmne/zapfast zapfast
cd zapfast
cargo install --path .
zapfast
```

On Linux, `cargo install` puts the binary on your `PATH` but does not add a
launcher entry. To get one (ZapFast under your application launcher, with its
own icon and no terminal), install a release build instead:

```sh
cargo build --release --locked
packaging/install-user.sh
```

This installs the binary, the icon, and a desktop file under `~/.local` (or
`$PREFIX`) and writes that desktop file's `Exec=` as the quoted full path to the
binary it just installed, so the entry works even when `~/.local/bin` is not on
the session's `PATH`. `install-user.sh` is Linux-only; on macOS and Windows a
source build has no launcher integration.

The build needs a C/C++ toolchain, CMake, and Perl (for the bundled
OpenSSL); `rust-toolchain.toml` pins the Rust version. On Linux it also needs
egui's development libraries, ALSA, and PipeWire with clang (the PipeWire
output's bindings are generated at build time). libopus and the H.264 decoder
build from source. On Arch Linux:

```sh
sudo pacman -S --needed alsa-lib pipewire clang libxkbcommon wayland cmake perl
```

On Debian 13, Ubuntu 24.04 or newer:

```sh
sudo apt install build-essential cmake perl clang libclang-dev libasound2-dev libpipewire-0.3-dev libxkbcommon-dev libwayland-dev libgl1-mesa-dev
```

With Nix, `nix develop` provides the toolchain and every native dependency,
and `nix run .#zapfast` builds and runs it.

The packaged desktop entry is `packaging/applications/zapfast.desktop`;
`packaging/install-user.sh` derives the user copy above from it.

## Link with your phone

ZapFast links as a companion device, like WhatsApp Web. Before requesting a code
for a new account, choose **Recent** (the default) for a quicker setup, or
**Full** and select a UTC date from 1 to 1,095 days ago. The one-, two-, and
three-year presets fill the same exact date field. Your phone decides how much
it can supply, so the selected period is a request, not a guarantee. Longer
periods can take more time and disk space.

The selected date is also saved as a resumable archive request pass. ZapFast
starts with recently active chats, follows only replies matched to its own
requests, and keeps its place across restarts. Existing linked accounts can
choose or change the date under **Settings > History archive** without
relinking. Choosing **Recent** cancels a dated pass without deleting messages
already archived.

The syncing indicator shows recent transfer activity. It can clear during a
quiet period while the phone still has more history to send. A reported
percentage belongs to a particular transfer type, not to verified coverage
of the requested years. The phone's paused message can also appear while
history arrives intermittently; check whether older messages continue to appear.

Local log lines beginning with `[HISTORY]` distinguish `flow=initial` chunks
from `flow=on_demand` replies to history requests, including poll recovery.
They contain only a local account number, fixed stage labels and counts.
`received` records delivery; `applied` means the app finished processing the
chunk, including any write errors reported separately. It is not a read-back
verification or a promise that the requested period is complete. `quiet`
records inactivity, and `request_sent`, `request_failed` and `request_timeout`
describe older-message page requests. Entry counts include repeated messages.

Click **Continue to linking**, then either:

- scan the QR code with your phone (WhatsApp, **Settings**, **Linked
  devices**, **Link a device**). The phone's camera app also reads the code
  and opens **Linked devices** in WhatsApp, or
- click **Link with phone number** and enter the eight-character code on your
  phone.

The link survives restarts. Your phone does not need to stay on the same
network or be online to read messages already stored in ZapFast.

Several numbers can stay linked in one window. Click your own picture at the
bottom of the rail beside the chat list to switch between them or to add
another (**Settings >
Account > Add account** works too); a dot on it means another number has
unread chats. `Ctrl+Shift+A` (Command+Shift+A on macOS) switches to the next
number. Each number keeps its own keys, archive, media, and
notification, receipt, download, and wallpaper settings.

## Message history

After linking, the phone sends the history available for the requested profile.
The chat list can appear before messages finish loading. ZapFast stores
new messages in its own archive. When you scroll past the stored history,
ZapFast asks your phone for older messages. The phone must be online. A phone
may leave a request unanswered or return less than the selected period. The
settings show a request pass as running, waiting, paused, finished, or needing
attention. Finished means every eligible chat reached the date or the phone
explicitly said it had nothing older. A retry limit remains **Needs attention**
until you choose **Retry**; it is not reported as complete. This records what
the request pass proved. It cannot guarantee that WhatsApp supplied every
message inside the period.

Media recovery is separate. When enabled, ZapFast downloads attachments in the
same selected range, newest first, within the storage budget. Expired files may
optionally be requested from the phone under their own daily limit. Files that
need the phone, are unavailable, were cancelled, or exceed the automatic size
policy remain separate in the displayed counts. Large-file recovery is an
explicit opt-in and allows up to 2 GiB per file.

## Try it in your own chat

Use WhatsApp's **Message yourself** chat to try messages, reactions, edits,
voice messages, and attachments privately.
