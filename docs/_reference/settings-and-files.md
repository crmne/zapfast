---
title: Settings & Files
description: Settings and paths for the archive, configuration, caches, and logs.
nav_order: 0
---

## File locations

ZapFast follows each platform's conventions. Each linked number has its own
folder, `<id>` below, so files, caches, and keys never mix between numbers.
On Linux:

| What | Where | Notes |
| --- | --- | --- |
| Settings | `~/.config/zapfast/settings.json` | JSON, safe to edit; the app lock password and the locked-chats code are kept only as salted verifiers |
| Account list | `~/.config/zapfast/accounts.json` | Which numbers are linked here and which one is showing |
| Session keys | `~/.local/state/zapfast/accounts/<id>/session.db` | Deleting it unlinks that number |
| Message archive | `~/.local/state/zapfast/accounts/<id>/archive.db` | Encrypted; the only copy of history WhatsApp will not send again |
| Per-number settings | `~/.local/state/zapfast/accounts/<id>/settings.json` | Notifications, receipts, typing, automatic downloads, the last open chat, and the wallpaper |
| Stickers and packs | `~/.local/state/zapfast/accounts/<id>/stickers/` | Plain WebP files; each pack is a folder |
| Wallpaper image | `~/.local/state/zapfast/accounts/<id>/wallpaper.jpg` | Copy of the chosen picture (or `.png`, `.webp`, `.gif`) |
| Attachments, avatars | `~/.cache/zapfast/accounts/<id>/` | Safe to delete; available files download again when viewed |
| Last run's log | `~/.local/state/zapfast/zapfast.log` | `--verbose` for more |
| Crash log | `~/.local/state/zapfast/panic.log` | Safe to delete |

Back up the archive if you need its history. WhatsApp sends only recent
history to a new device, although ZapFast can request some older messages from
the phone. Clearing the media cache makes ZapFast download attachments again.
Expired attachments may still be available through the phone.
**Settings > Files > Change…** sends new downloads to another folder, leaving
earlier ones in place.

On macOS, settings, state, and the logs are in
`~/Library/Application Support/me.paolino.zapfast` and the caches in
`~/Library/Caches/me.paolino.zapfast`. On Windows, settings are in
`%APPDATA%\paolino\zapfast\config`, state and the logs in
`%LOCALAPPDATA%\paolino\zapfast\data`, and the caches in
`%LOCALAPPDATA%\paolino\zapfast\cache`.

On first start, ZapFast moves the corresponding `fastsapp` directories (or
`fastwhatsapp` from earlier versions), including the session, archive, saved
stickers, and window state. Existing ZapFast directories are never overwritten.
Quit FastsApp first; launching ZapFast while it is running brings the existing
window forward. A single-number setup from before 0.19 moves into
`accounts/1/` on first start, keyring key included; if that cannot finish (a
locked keyring, or a folder already in the way), ZapFast stops without moving
anything and says why in its log.

On Linux and macOS, ZapFast restricts its configuration, state, and cache
directories to your user (`0700`) and stops at startup if it cannot. Windows
uses the permissions inherited from your user profile.

## Archive encryption

The message archive is a SQLCipher database. Its key is a random 256-bit
secret in the OS credential store: Secret Service on Linux (GNOME Keyring, or
KeePassXC with Secret Service enabled), Keychain on macOS, or Windows
Credential Manager. Back up both the archive and its keyring entry: copying
only `archive.db` to another computer is not enough.

If the keyring is locked or unavailable, unlock it and click **Retry**;
ZapFast keeps the archive intact and waits before connecting. It never saves
a replacement plaintext archive.

A missing key is different from a locked keyring. Restore the original
credential store or profile location if you can. Deleting the archive or
creating new credentials will not help: neither can decrypt the existing
archive. If the key cannot come back, **Start over…** renames the unreadable
archive to `archive-unreadable-<date>.db`, forgets the linked session, and
shows the linking screen; linking again brings recent history back from your
phone. Remove the old ZapFast entry under **Linked devices** on the phone
afterwards.

Only the archive and its SQLite journal are encrypted. Session keys,
downloaded media, profile pictures, stickers, and settings are ordinary files;
use full-disk encryption for those, and for swap and backups. Keyring
unlocking does not protect against software running as you while you are
logged in.

When asking for help, report the OS, the app version, whether the profile was
moved or restored, and the error text with personal paths removed. Never
attach the archive, keys, or full logs from older releases.

## Settings

Changes on the Settings page are saved to `settings.json` immediately. The
search field at the top (`Ctrl+F`, Command+F on macOS) finds a setting by its
name or description, in the interface language or in English.

**Appearance**

- **Theme**: dark, light, follow the system, or a local JSON palette from the
  themes folder. See [Making a theme]({{ '/themes/' | relative_url }}).
- **Wallpaper**: WhatsApp's light and dark chat wallpaper colours, with or
  without doodles.
- **Zoom**: interface scale, also `Ctrl+Plus`, `Ctrl+Minus` and `Ctrl+0`.
- **Language**: the interface language, or **Auto** to follow the system.
  Brazilian Portuguese, German, Spanish, Italian, French, Russian, Simplified
  Chinese, Traditional Chinese, Turkish, and Indonesian are available; otherwise English.
  Message contents, names, and logs are never translated.

**Chats**

- **Enter sends**: when off, Enter adds a line and `Ctrl+Enter` (Command+Enter
  on macOS) sends.
- **Download files automatically**: attachments up to 64 MiB download as they
  come into view. When off, click one to download it. Visible stickers still
  download automatically. The same 64 MiB limit applies to manual downloads.
- **Keep chats archived** (development builds): on by default. When off, a new
  message, received or sent, brings an archived chat back to the list. Applies
  to all accounts here; it does not read or change the phone's own setting.
- **Pause other media while recording or playing**: pause music and videos in
  other apps while you record, or while a voice message, audio, or video plays
  with sound, and resume them afterwards. Linux and Windows only.
- **Locked chats code**: the local code that opens the **Locked** tab. It hides
  chats and adds no encryption beyond the encrypted message archive.

**Notifications**

- **Desktop notifications**: for chats you are not looking at. Muted chats stay
  quiet.
- **Message sound** and **Mention sound**: Pidgin's sounds, the system sound,
  none, or an audio file.
- **Play sounds for group messages**: when off, only mentions and replies to
  you make a sound in groups.
- **Limit repeat notifications**: off by default. Choose **Direct messages**,
  **Groups**, or **Direct messages and groups**. A limited chat sends one
  notification (and its sound), then stays quiet for 10 minutes. Opening the
  chat starts over. In groups, a mention of you or a reply to you always
  notifies. Settings for this account.

**Privacy**

- **Send read receipts**: the blue ticks others see, subject to the account
  setting below.
- **Show when you are typing**: send typing and recording state.
- **Last seen**, **Online**, **Profile photo**, **About**, **Groups**, **Read
  receipts**, **Calls**: your WhatsApp account privacy, stored on WhatsApp's
  servers and shared with your phone. They can be changed while connected.
- **App lock**: hide ZapFast behind a password. See [App lock](#app-lock).

**System**

- **Keep running when the window closes**: keep ZapFast linked in the tray.
- **Start at login**: start in the tray without a window, where the platform
  supports it.
- **Check for updates**: ask GitHub once a day whether a newer release exists.
- **Download updates automatically**: download and verify a new release in the
  background; restarting stays your choice. Package managers and Flatpak update
  ZapFast themselves.
- **Proxy**: for the WhatsApp connection, media, profile pictures, GIF search,
  Signal sticker imports, and update checks. It accepts `socks5h://host:port`
  (the proxy resolves names, as Tor expects), `socks5://host:port`, and
  `http://host:port`, each with an optional `user:password@`; a bare
  `host:port` is an HTTP proxy. Changing it reconnects at once. Empty uses
  `ALL_PROXY` or `HTTPS_PROXY` from the environment and honors `NO_PROXY`.
- **GIPHY API key**: for GIF search, unless the build includes one. Set
  `ZAPFAST_GIPHY_KEY` at compile time to include a default key. The earlier
  `FASTSAPP_GIPHY_KEY` remains a fallback for existing builds.

**Account** edits your WhatsApp name, About, and picture, and unlinks this
computer. **Files** shows the archive, the downloads folder (which you can
change; earlier downloads stay where they are), and this run's log.

Some choices are made where they are used and remembered in `settings.json`:
the shortcut hints bar under the composer (its × hides it, and **Show shortcut
hints under the message box** in the Keyboard shortcuts dialog brings it back),
**Also save to your phone's contacts** in the new-contact dialog, voice
playback speed, and the chat list and search pane widths.

Development builds also remember the window's size, position, and maximized
state in `settings.json`, including when reopening from the tray or a
notification. On Wayland the compositor controls placement, so ZapFast keeps
the size without choosing the position. These window changes and **Keep chats
archived** are not yet included in 0.19.0.

Labels always get a chip each, in a row under the filter chips, once a label
exists. They are kept in the message archive, next to your chats, and never
leave this computer. Sender pictures appear in groups only, and names from your
address book come before public profile names. Hiding the chat list (`Ctrl+B`)
collapses it to a column of avatars with unread badges.

Settings from earlier versions that no longer exist are ignored and dropped
the next time settings are saved. The two earlier media pause switches become
the one above: it stays on only if both were on.

## App lock

Like WhatsApp Web's screen lock, **Settings > Privacy > App lock** hides
ZapFast behind a password of at least six characters. ZapFast then starts
locked and locks again after 1 minute, 15 minutes (the default), or 1 hour
without input, a choice under **Lock after**; time hidden in the tray counts.
**Lock ZapFast** in the tray menu and `Ctrl+Shift+L` lock it at once.

While locked, the window shows only the lock screen. Messages keep arriving
but stay unread, and notifications say only "New message", without the chat,
sender, text, or picture. Clicking one opens the message after you unlock.
Voice messages and videos stop, and a recording in progress is discarded.

Wrong passwords make the next try wait, up to half a minute. **Forgot
password? Unlink this computer** is the only way back in without it: it
unlinks this computer and deletes the chats stored here. Any unlink, including
one from your phone, turns the app lock off.

The app lock keeps people using this computer out of your chats; it encrypts
nothing beyond the archive, and someone who can edit your files can remove it
from `settings.json`, which keeps only a salted PBKDF2 verifier of the
password. It is independent of the locked-chats code: unlocking one never
opens the other.

## Updates

With **Check for updates** on, ZapFast asks GitHub once a day for a newer
release. **Update** in the banner downloads and verifies it, and **Restart to
update** installs it when convenient. Downloads are checked against the
release's SHA-256 checksums, whose manifest must carry a valid Ed25519
publisher signature; a missing or invalid signature stops the update. The
updater keeps a backup and restores it if the new version cannot start.
Release builds also carry GitHub provenance attestations
(`gh attestation verify FILE -R crmne/zapfast`).

The in-app updater handles portable downloads (keep `zapfast-portable.txt`
beside the executable), the Windows installer, and the macOS app in
Applications. AUR, DEB, RPM, Flatpak, Nix, Cargo, and Homebrew installations
update through their package manager; an AppImage is replaced by downloading
the new one.

## The log

Each run replaces `zapfast.log` and records warnings and errors, never
message contents, phone numbers, or keys. Include the end of this file when
reporting an issue. **Settings > Files > Log > Open** shows it.
