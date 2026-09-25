---
title: Settings & Files
description: Settings and paths for the archive, configuration, caches, and logs.
nav_order: 0
---

## File locations

ZapFast follows each platform's conventions. On Linux:

| What | Where | Safe to delete? |
| --- | --- | --- |
| Settings | `~/.config/zapfast/settings.json` | Yes, you lose preferences |
| Message archive | `~/.local/state/zapfast/archive.db` | Yes; only history available from WhatsApp can be restored |
| Session keys | `~/.local/state/zapfast/session.db` | Yes; you must link again |
| Attachments | `~/.cache/zapfast/media/` | Yes; available files download again when viewed |
| Profile pictures | `~/.cache/zapfast/avatars/` | Always |
| Stickers | `~/.cache/zapfast/stickers/` | Always |
| GIF search stills | `~/.cache/zapfast/gifs/` | Always |
| Last run's log | `~/.local/state/zapfast/zapfast.log` | Always |
| Crash log | `~/.local/state/zapfast/panic.log` | Always |

Back up the archive if you need its history. WhatsApp sends only recent
history to a new device, although ZapFast can request some older messages from
the phone. Clearing the media cache makes ZapFast download attachments again.
Expired attachments may still be available through the phone.

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
window forward.

## Settings

Changes on the Settings page are saved to `settings.json` immediately. The
search field at the top (`Ctrl+F`, Command+F on macOS) finds a setting by its
name or description, in the interface language or in English.

**Appearance**

- **Theme**: dark, light, follow the system, or a local JSON palette from the
  themes folder.
- **Wallpaper**: WhatsApp's light and dark chat wallpaper colours, with or
  without doodles.
- **Zoom**: interface scale, also `Ctrl+Plus`, `Ctrl+Minus` and `Ctrl+0`.
- **Language**: the interface language, or **Auto** to follow the system.

**Chats**

- **Enter sends**: when off, Enter adds a line and `Ctrl+Enter` (Command+Enter
  on macOS) sends.
- **Download files automatically**: files up to 64 MiB, stickers included,
  download as they come into view. When off, click one to download it.
- **Preview links**: read a page's own title, description, and picture for a
  link WhatsApp sent without preview metadata, which happens when the sender
  turned previews off on their phone. The page is fetched from this computer
  and only the address the sender wrote is requested; the conversation is not
  sent anywhere. When off, such links stay plain text.
- **Pause other media while recording or playing**: pause music and videos in
  other apps while you record, or while a voice message, audio, or video plays
  with sound, and resume them afterwards. Linux and Windows only.
- **Locked chats code**: the local code that opens the **Locked** tab. It hides
  chats; it does not encrypt them.

**Notifications**

- **Desktop notifications**: for chats you are not looking at. Muted chats stay
  quiet.
- **Message sound** and **Mention sound**: Pidgin's sounds, the system sound,
  none, or an audio file.
- **Play sounds for group messages**: when off, only mentions and replies to
  you make a sound in groups.

**Privacy**

- **Send read receipts**: the blue ticks others see, subject to the account
  setting below.
- **Show when you are typing**: send typing and recording state.
- **Last seen**, **Online**, **Profile photo**, **About**, **Groups**, **Read
  receipts**, **Calls**: your WhatsApp account privacy, stored on WhatsApp's
  servers and shared with your phone. They can be changed while connected.

**System**

- **Keep running when the window closes**: keep ZapFast linked in the tray.
- **Start at login**: start in the tray without a window, where the platform
  supports it.
- **Check for updates**: ask GitHub once a day whether a newer release exists.
- **Download updates automatically**: download and verify a new release in the
  background; restarting stays your choice. Package managers and Flatpak update
  ZapFast themselves.
- **Proxy**: for WhatsApp, media, link previews, and updates. Empty uses
  `ALL_PROXY` or `HTTPS_PROXY`.
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

Labels always get a chip each, in a row under the filter chips, once a label
exists. They are kept in the message archive, next to your chats, and never
leave this computer. Sender pictures appear in groups only, and names from your
address book come before public profile names. Hiding the chat list (`Ctrl+B`)
collapses it to a column of avatars with unread badges.

Settings from earlier versions that no longer exist are ignored and dropped
the next time settings are saved. The two earlier media pause switches become
the one above: it stays on only if both were on.

## The log

Each run replaces `zapfast.log` and records warnings and errors. Include the
end of this file when reporting an issue.
