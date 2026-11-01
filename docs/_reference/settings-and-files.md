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
| Per-number settings | `~/.local/state/zapfast/accounts/<id>/settings.json` | Notifications, receipts, typing, automatic downloads, background history, the last open chat, the chat-list chip or label, and the wallpaper |
| Stickers and packs | `~/.local/state/zapfast/accounts/<id>/stickers/` | Plain WebP files; each pack is a folder |
| Wallpaper image | `~/.local/state/zapfast/accounts/<id>/wallpaper.jpg` | Copy of the chosen picture (or `.png`, `.webp`, `.gif`) |
| Attachments, avatars | `~/.cache/zapfast/accounts/<id>/` | Safe to delete; available files download again when viewed |
| Last run's log | `~/.local/state/zapfast/zapfast.log` | `--verbose` for more |
| The run before's log | `~/.local/state/zapfast/zapfast.previous.log` | Kept for one more start |
| Crash log | `~/.local/state/zapfast/panic.log` | Safe to delete |

Back up the archive if you need its history. WhatsApp sends only recent
history to a new device, although ZapFast can request some older messages from
the phone. Clearing the media cache makes ZapFast download attachments again.
Expired attachments may still be available through the phone.
A download is reported as complete only after its local path is recorded in
the archive. If that write fails, the attachment shows a failure instead;
the downloaded file is kept in its destination.
**Settings > Files > Storage used** shows how much space the account on screen
has in its media cache, split into images, videos and GIFs, stickers, and other
files. **Clear…** there deletes that account's cached attachments after asking:
its messages, its other caches, and the other accounts' files stay, and so do
files in a download folder you chose.
**Settings > Files > Change…** sends new downloads to another folder, leaving
earlier ones in place. New downloads use one fixed layout inside that folder:
`<account>/<chat name>/file`, where the account folder is the account's phone
number, such as `+15550001111` (`Account 1` until the number is known). Chat
names stay readable, with filesystem-unsafe characters replaced and long names
shortened. When two chats of one account would share a folder, the first keeps
it and the others become `Name (2)`, `Name (3)`, and so on; each chat keeps its
folder. New downloads use the current name; existing files are not moved.
The downloaded file itself goes there, without a second copy in the cache.
If the destination is unavailable, downloads fall back to the account's normal
cache as before. This also applies when the folder cannot accept new files or
publishing the downloaded file fails. Restarting removes unfinished downloads
from that account's chat folders, without following linked folders.
If an account or chat folder is a symbolic link, new downloads use the cache
instead.
Documents keep the name they were sent with; a
file already in the folder is never replaced, and the download becomes
`name (1).pdf`, `name (2).pdf`, and so on. Characters a file system refuses
become `_`.

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
anything and says why in its log. If `accounts/1/` already holds a linked
session, ZapFast uses it and leaves any older files beside it alone; an older
ZapFast still installed can leave such files.

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

- **Theme**: dark, light, follow the system, Messages, or a local JSON
  palette from the themes folder. See [Making a theme]({{ '/themes/' | relative_url }}).
  **Messages** follows the system's light or dark appearance in macOS's
  greys, with bubbles shaped, spaced and coloured as Messages on the Mac
  draws them (yours in iMessage blue, shading deeper down the window), the
  Mac's system blue for buttons and highlights, a frosted header that
  messages scroll under, and photos without a bubble. Its sidebar and
  conversation show the desktop through the window where the platform
  offers a material: AppKit's on macOS, Mica on Windows 11 22H2 and later,
  and Hyprland's blur on Linux. Elsewhere, including other Linux desktops,
  where an unblurred desktop behind the sidebar would cost its text
  contrast, and in a window opened before choosing it on Windows or Linux,
  the window is opaque in the theme's colours; it turns translucent from
  the next window.
- **Wallpaper**: WhatsApp's light and dark chat wallpaper colours, with or
  without doodles.
- **Zoom**: interface scale, also `Ctrl+Plus`, `Ctrl+Minus` and `Ctrl+0`.
- **Language**: the interface language, or **Auto** to follow the system.
  Brazilian Portuguese, German, Spanish, Italian, French, Russian, Simplified
  Chinese, Traditional Chinese, Turkish, and Indonesian are available; otherwise English.
  Message contents, names, and logs are never translated.
- **Custom title bar** (Windows and Linux, off by default): ZapFast draws
  its own title bar instead of the desktop's. The window buttons sit in the
  top corner of whichever pane is there (the open chat's header, the info
  panel, Settings, the Calls page, the call screen), on the left or the
  right as GNOME's button layout says, or Windows' three square buttons;
  the headers drag the window and a double-click maximizes or restores it,
  and the window's edges resize it. Turning it on or off replaces the
  window at once, keeping the open chat and any call. It is off by default
  because some Intel graphics drivers draw a frameless window with a black
  band at the top, with clicks landing below where they are drawn. macOS
  always keeps its own traffic lights.

**Chats**

- **Enter sends**: when off, Enter adds a line and `Ctrl+Enter` (Command+Enter
  on macOS) sends.
- **Download files automatically**: attachments up to 64 MiB download as they
  come into view. When off, click one to download it. Visible stickers still
  download automatically. Larger attachments download only when clicked.
- **Fetch older history**: off by default, and set for
  each number. When on, ZapFast asks the phone for older messages without you
  scrolling: the open chat first, then pinned chats and the ten most recently
  active ones (never channels). It sends at most one request every twenty
  seconds, and only while no other history request is waiting. Each chat is
  asked for at most five pages each time ZapFast runs, and a chat the phone
  does not answer goes to the back of the queue. Unanswered requests back off
  for that chat only; other chats keep the normal twenty-second gap. After
  three unanswered requests in a row, that chat's background recovery pauses
  until one of those requests gets a late reply, or ZapFast reconnects or
  restarts. This does not mark history complete or prevent manual requests.
  A matched background reply resets that chat's timeout count, even if late.
  Requests that cannot be sent still back off the whole account. Pages fetched
  this way go into the archive without moving the view. A chat the phone says
  has nothing older is not asked again.
- **Keep chats archived**: on by default. When off, a new message, received or
  sent, brings an archived chat back to the list. Applies to all accounts here
  whose phone has not sent its own **Keep chats archived**; once a phone has,
  ZapFast follows the phone for that account. It does not change the phone's
  setting.
- **Pause other media while recording or playing**: pause music and videos in
  other apps while you record, or while a voice message, audio, or video plays
  with sound, and resume them afterwards. Linux and Windows only.
- **Locked chats code**: the local code that opens the **Locked** tab. It hides
  chats and adds no encryption beyond the encrypted message archive.

**Notifications**

- **Desktop notifications**: for chats you are not looking at. Muted chats stay
  quiet. The bell beside each number in the account switcher turns the same
  setting off and on for that number without switching to it.
- **Show message content in notifications**: on by default. When off, every
  notification says only "New message" from ZapFast, without the chat, the
  sender, the text, or a picture, as behind the app lock, and plays only the
  message sound. Clicking one still opens the message. Applies to every
  account.
- **Message sound** and **Mention sound**: Pidgin's sounds, the system sound,
  none, or an audio file. On macOS, Pidgin's sounds and the system sound play
  with the notification, so Do Not Disturb, Focus, and the notification
  settings for ZapFast silence them; an audio file of your own plays
  regardless.
- **Play sounds for group messages**: when off, only mentions and replies to
  you make a sound in groups.
- **Limit repeat notifications**: off by default. Choose **Direct messages**,
  **Groups**, or **Direct messages and groups**. A limited chat sends one
  notification (and its sound), then stays quiet for 10 minutes. Reading the
  chat, here or on another device, starts over. In groups, a mention of you or a reply to you always
  notifies. Settings for this account.

**Privacy**

- **Send read receipts**: the blue ticks others see, subject to the account
  setting below. When account read receipts are off, read ticks are also hidden
  on outgoing direct messages in the conversation, chat list, and Message info,
  as on the phone. Group receipts and voice-message played receipts are
  unaffected.
- **Show when you are typing**: send typing and recording state.
- **Last seen**, **Online**, **Profile photo**, **About**, **Groups**, **Read
  receipts**, **Calls**: your WhatsApp account privacy, stored on WhatsApp's
  servers and shared with your phone. They can be changed while connected.
- **App lock**: hide ZapFast behind a password. See [App lock](#app-lock).

**System**

- **Keep running when the window closes**: keep ZapFast linked in the tray.
- **Start at login**: start in the tray without a window, where the platform
  supports it. If the desktop's tray is not up yet, ZapFast waits up to 30
  seconds for it, then opens the window, since a desktop without a tray could
  not bring it back. On macOS this is a login item under System Settings >
  General > Login Items, which macOS may ask you to allow there; inside a
  Flatpak the desktop's background portal asks first.
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
  A call's signaling goes through the proxy, but its audio and video do not:
  they travel over UDP straight to WhatsApp's call relays, which see this
  computer's address.
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
notification. The window opens at the same size whatever the interface's
zoom. On Wayland the compositor controls placement, so ZapFast keeps the size
without choosing the position. These window changes and **Keep chats
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

Each run starts `zapfast.log` afresh and records warnings and errors, never
message contents, phone numbers, or keys. Include the end of this file when
reporting an issue. **Settings > Files > Log > Open** shows it. The run before
is kept beside it as `zapfast.previous.log`: if ZapFast froze and you had to
restart it, that file holds what happened.

During a call the log also records the call's numbers every few seconds: the
video rate, loss and keyframes in each direction, and the audio devices'
health. They are counts and sizes only.

### Saving a call's video for diagnosis

To work on a video call's picture quality, start ZapFast with
`ZAPFAST_CALL_VIDEO_DUMP` set to a folder. Each video call then writes the
video it sends and the video it receives there, as raw H.264
(`<time>-<call>-sent.h264` and `-received.h264`), which `ffmpeg` and media
players read. Nothing is written without the variable. The files are the
picture of both people on the call, so only set it for a call whose other side
agreed, and delete the files when you are done.

### Saving a call's audio for diagnosis

To work on echo cancellation, start ZapFast with `ZAPFAST_CALL_AUDIO_DUMP` set
to a folder. Each call's microphone stream then writes four files there,
`<time>-<stream>-reference.wav` (the other side's voice as your speaker played
it), `-microphone.wav` (your microphone before any processing), `-output.wav`
(what was sent after echo cancellation, noise suppression and level control),
all mono 16 kHz, and `-order.csv`, which lets a developer replay the recording
through the echo canceller exactly as the call ran it
(`cargo run --release --example aec_replay -- <folder>/<time>-<stream>-reference.wav`,
with the options listed at the top of `examples/aec_replay.rs`).
Nothing is written without the variable. The files are both people's voices,
so only set it for a call whose other side agreed, and delete the files when
you are done.
