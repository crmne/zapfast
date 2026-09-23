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

Changes on the Settings page are saved to `settings.json` immediately:

- **Theme**: light, dark, or follow the system.
- **Enter sends**: swap Enter and Shift+Enter.
- **Download attachments automatically**: download non-sticker files up to 64
  MiB when they enter view, or only when clicked. Visible stickers download
  automatically up to the same limit.
- **Show sender pictures**: avatars next to group messages.
- **Names from your address book**: use contact names everywhere. When off,
  prefer public profile names.
- **Send read receipts**: the blue ticks others see.
- **Keep running in the background**: keep ZapFast in the tray when the
  window closes.
- **Notifications**: use desktop notifications with the chat picture.
- **Check for updates**: ask GitHub once a day whether a newer release exists.
- **Show labels as chips**: give each label its own chip, in a row under the
  filter chips. With it off, labels share one **Labels** chip. Labels
  themselves are kept in the message archive, next to your chats, and never
  leave this computer.
- **GIPHY API key**: required for GIF search unless the build includes one.
  Set `ZAPFAST_GIPHY_KEY` at compile time to include a default key.
  The earlier `FASTSAPP_GIPHY_KEY` remains a fallback for existing builds.

## The log

Each run replaces `zapfast.log` and records warnings and errors. Include the
end of this file when reporting an issue.
