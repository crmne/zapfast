---
title: Using ZapFast
description: Send messages and use attachments, interactive messages, voice messages, and keyboard shortcuts.
redirect_from:
  - /using-fastsapp/
nav_order: 3
---

## Writing

Enter sends and Shift+Enter adds a line. You can swap them in Settings.
`*bold*`, `_italic_`, `~strike~`, and ```` ```monospace ```` ```` format
like WhatsApp, and a message of nothing but emoji shows large.
Mentions in a group are written with `@`; the smiley opens emoji
(searchable), GIFs, and stickers, including the stickers used on the
phone.

Right-click a message to reply, react with any emoji, edit, forward, delete, or
check when it was sent, delivered, and read. The reaction row has a **+** that
opens the full emoji picker. Hover over a reaction to see who added it.
Editing uses the composer. Press Escape to cancel.

Double-click beside a message, or on its edge, to reply to it. A double-click
on its text still selects the word.

## Stickers

Right-click a sticker in a chat or the picker to save it. Saved stickers
appear in the **Saved** row. To import a pack, click **Find packs**, copy a
`signal.art` link from [signalstickers.org](https://signalstickers.org), and
paste it into the field. You can also open a `.wastickers` file. Animated
stickers remain animated and play on hover. Use the delete button beside a
pack to remove it. Packs are stored as WebP files on your computer.

## Attachments

Paste a picture, drop files on the window, or select them with the paperclip.
They stay above the composer until you send them, with the typed text as a
caption. Press Escape or click a file's close button to remove it. Incoming
attachments up to 64 MB download when they enter view, or on click if automatic
downloads are off. If an attachment has expired, ZapFast asks your phone to
upload it again.

## Interactive messages

Business templates and button messages show their image above the formatted
text, with options in separate rows below the timestamp. Lists and carousel
text are readable too. You can select the message body, use **Copy text** to
include its option labels, and find these messages through search.

- **Web links** have an external-link icon. Click one to open it in your browser,
  or focus it with the keyboard and press Enter.
- **Phone options** have a phone icon and muted text. Reply buttons, list choices,
  calls, and forms must be used on your phone. Hover over an option to see its
  explanation. ZapFast does not send a response when you click it.
- **Replies from your other devices** appear as ordinary replies, with a quote
  when the original message is included.

Images follow the same automatic-download setting, size limit, and retry
behavior as other photos. Click a downloaded image to open it.

| Dark theme | Light theme |
| --- | --- |
| ![Synthetic business message with an image, three phone options, a quoted reply, and a website link in the dark theme](/screenshot-interactive-media.png) | ![The same synthetic interactive messages in the light theme](/screenshot-interactive-light.png) |

Both screenshots use offline demo content.

Previously unsupported messages are recovered automatically from the local
archive when their original data is available and they have not been edited.
You do not need to link again. Edited messages keep their current text.

Embedded videos and documents, carousel images, and templates containing only
a reference to server-side text still need your phone. A **More content on your
phone** note marks content ZapFast cannot display. Interactive messages cannot
yet be forwarded from ZapFast.

## Voice messages

Voice messages play in the chat with a seekable waveform. The button beside
the waveform cycles the playback speed between 1x, 1.5x, and 2x, and the choice
is remembered for later messages. The speaker's pitch stays the same at every
speed. The first play sends
a played receipt. When the composer is empty, the send button becomes a
microphone. Press Enter or the send button to send the recording, or Escape or
the delete button to discard it. ZapFast raises the volume of quiet recordings.
Starting a reply before recording includes the quoted message.

## Copying

Select and copy any message text. A selection across messages uses WhatsApp's
sharing format:

```
[18:21, 8/30/2026] Ada Lovelace: Hello from France!
[18:27, 8/30/2026] You: Sure, I will take a look
```

## Chats

The search bar finds chats by name, number, or latest message; searches all
messages stored on this computer; and finds contacts without an existing chat.
Click a message result to jump to it, or a contact to start a chat. Use
`Alt+↑/↓` to switch chats without leaving the composer.

The chips under the search bar narrow the list to **Unread**, **Private**
(one-to-one chats), or **Groups**. A chip with unread chats shows how many it
has. Click the active chip again, or **All**, to see every chat. The
filter applies only to this list: search and the archive still show everything,
and it resets when ZapFast restarts.

Right-click a chat to pin, archive, or mute it for eight hours, one week, or
indefinitely. These changes also apply on your phone. Click the chat header to
see its picture, number, and group members.

## Notifications and the tray

Closing the window keeps ZapFast linked in the tray. Click the tray icon or
launch the app again to reopen it. Notifications show the chat picture and open
the chat when clicked. Muted chats do not send notifications. You can change
both settings.

Press `Ctrl+/` or click the keyboard button under the composer to list all
shortcuts.
