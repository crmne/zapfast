---
title: Using ZapFast
description: Send messages, use attachments, interactive messages, and voice messages, make voice and video calls, and learn the keyboard shortcuts.
redirect_from:
  - /using-fastsapp/
nav_order: 3
---

## Calling

The phone icon in a chat's header starts a voice call: the chat takes over the
window, and the call's length only starts counting once the other side has
really answered. The camera icon starts a video call instead. A call you are
receiving takes over the window by itself, whoever you were writing to, with the
caller's name and picture, a line saying whether it is a voice or a video call,
and **Accept** and **Decline**. Names come from your address book or the public
profile name, exactly as they do in the chat list. A desktop notification with
the caller's name goes out as well, so a window hidden in the tray or behind
another program does not hide the call; clicking it brings ZapFast up with the
call waiting. A chat you have muted, archived, or locked keeps its calls as
quiet as its messages.

Along the bottom of a call are the controls: mute, camera, the speaker button
that opens the device pickers, and the red button that hangs up. Mute goes
through the calling library's own mute, so the other side is told and the audio
stream is not restarted. **Microphone**, **Speaker**, and **Camera** list the
devices this computer really has; picking one applies to ZapFast's call and
never to your system defaults, and the choice is remembered for the next call. A
device that has gone away since it was picked — a headset switched off, a camera
unplugged — falls back to the system default and the call screen says so.

Turning the camera off leaves the call and its audio running; so does a camera
that stops delivering, which is reported rather than ending the call. A video
call shows the other side's picture as large as it fits without stretching it,
with your own camera in the corner. If their phone announces a rotation, it is
applied, so a phone held upright stays upright here.

**Back to the chat** in the top-left corner of a call puts you back in the
conversation with the call still running in a bar at the bottom of the window,
showing its length and a **Return to the call** button. Hanging up is the only
thing that ends a call. If you close the window to the tray, the call keeps
running there.

When a call is over, both sides' outcomes are recorded in the local archive:
the **Calls** view in the sidebar lists every call with its direction, how long
it lasted, and how it ended, and the same entry appears in its chat with the
time. A call that was not answered has no length, because there was no call to
measure.

Calling needs PipeWire: ZapFast records through `pw-record` and plays through
`pw-play`, which the current Linux desktops provide. Video calls additionally
need a V4L2 camera under `/dev/video*`. Any camera the kernel exposes works,
including **OBS Virtual Camera** once OBS Studio has started it — start the
virtual camera in OBS, open a call's **Camera** picker, and choose it. That is
also how to show a window or a slide deck on a call: capture it in OBS and send
the virtual camera.

Sharing your screen directly from ZapFast is not available: the whatsapp-rust
revision it uses carries screen sharing for group calls only, so **Share
screen** is shown disabled rather than as a control that cannot work. OBS
Virtual Camera covers the same ground on Linux.

## Writing

Enter sends and Shift+Enter adds a line. Turn off **Enter sends** in Settings
to make Enter add a line and send with Ctrl+Enter (Command+Enter on macOS).
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

The sticker tab works like WhatsApp's: a row of tabs holds **Recent**
(the clock), **Favorites** (the star), each of your packs, and **+** for
adding more. ZapFast adds **Received** (the speech bubble) after Favorites.
Click a sticker to send it. Animated stickers play on hover.

**Recent** holds the stickers you sent, not the ones you received.
Right-click one to take it out of Recent here and on your phone.

**Received** holds the stickers people sent you that are already
downloaded, newest first, each once. Stickers already in Recent or
Favorites, and stickers from locked chats, stay out of it.

Right-click a sticker in a chat or the picker to add it to your
**Favorites**. Favorites stay in sync with your phone: a sticker you favorite
or unfavorite on either side follows on the other.

Type in the search field to find stickers by emoji (😂), by a word that names
an emoji ("laugh", "duck"), or by pack name. Stickers carry the emojis they
express in their metadata, as WhatsApp's own stickers do.

Under **+**, paste a `signal.art` link from
[signalstickers.org](https://signalstickers.org) (or click **Find packs**), or
open a `.wastickers` file, to import a pack. Signal packs keep each sticker's
emoji. Open a pack's tab and use its delete button to remove it. Packs are
stored as WebP files on your computer.

A WhatsApp sticker pack someone shares in a chat shows its name, publisher,
and size. Click **View stickers** to download and look at it, and **Add to my
stickers** to keep it as a pack here. To share one of your packs, open its tab
and click the send arrow beside its name: it goes to the open chat as a
WhatsApp sticker pack of up to 60 stickers, with each sticker's emojis.

You can also make packs of your own: type a name under **Make your own** and
click **Create pack**. Right-click any sticker and choose one of your packs in
the menu to add it; a check mark shows the packs it is already in, and
choosing a checked one takes it back out. A pack keeps its own copy of each
sticker, named by the sticker's content, so the same picture is added once
however many chats it came from. Deleting a pack removes its copies and leaves
your favorites and other packs alone.

To make a sticker from a picture, click **Make a sticker from a picture…**
under **+** and choose a PNG, JPEG, WebP, or GIF. Drag the square to choose
the part you want and use **Size** to resize it. A picture with a transparent
background keeps it unless you turn that off, and then the background becomes
white. Type the emojis that describe it, for search here and for WhatsApp's
sticker suggestions, then **Send** it to the open chat or **Add to favorites**.
The sticker is a 512 × 512 WebP under WhatsApp's 100 KB limit.

## Attachments

Paste a picture, drop files on the window, or select them with the paperclip.
They stay above the composer until you send them, with the typed text as a
caption. Press Escape or click a file's close button to remove it. Incoming
non-sticker attachments up to 64 MiB download when they enter view if automatic
downloads are on, or on click. Visible stickers download automatically up to the
same limit. If an attachment has expired, ZapFast asks your phone to
upload it again.

## Interactive messages

Business templates and button messages show their image above the formatted
text, with options in separate rows below the timestamp. Lists open a grouped
choice dialog, and carousels show separate image cards in a horizontal strip. You can select the message body, use **Copy text** to
include its option labels, and find these messages through search.

- **Web links** have an external-link icon. Click one to open it in your browser,
  or focus it with the keyboard and press Enter.
- **Reply buttons** send the selected response immediately, quoting the original
  message. ZapFast includes the option identifier so the business can recognize
  the choice. Legacy buttons, hydrated templates, and native-flow quick replies
  are supported.
- **Simple lists** open a dialog with section headings and descriptions. Select an
  item to send it. Legacy single-select lists and native-flow `single_select`
  menus are supported. Opening or dismissing the dialog does not send anything.
- **Copy-code buttons** copy the supplied code to your clipboard without sending
  a message.
- **Unavailable options** have a phone icon and muted text. Forms, payments,
  shopping flows, calls, and carousel choices need WhatsApp Web or your phone.
  Hover over an option to see its explanation. Unsupported or incomplete actions
  never send a guessed text response.

Replies require a connection and a writable chat, and cannot be sent to your own
outgoing cards. During a send, the card waits for its result before accepting
another reply. A failed send shows the normal failure status and allows another
attempt.

**Replies from your other devices** appear as ordinary replies, with a quote
when the original message is included.

Images follow the same automatic-download setting, size limit, and retry
behavior as other photos. Click a downloaded image to open it.

| Dark theme | Light theme |
| --- | --- |
| ![Synthetic business message with an image, three reply buttons, a quoted reply, and a website link in the dark theme](/screenshot-interactive-media.png) | ![The same synthetic interactive messages in the light theme](/screenshot-interactive-light.png) |

![Synthetic reply, list-selection, copy-code, and unavailable-form actions](/screenshot-interactive-actions.png)

All screenshots use offline demo content.

Previously unsupported messages are recovered automatically from the local
archive when their original data is available and they have not been edited.
Existing cards gain their supported actions too. You do not need to link again.
Edited messages keep their current text, and downloaded images stay available.

Embedded videos, documents, and templates containing only
a reference to server-side text still need another client. A **More content in
WhatsApp Web or on your phone** note marks content ZapFast cannot display.
Interactive messages cannot yet be forwarded from ZapFast.

### Lists, polls, and carousels

Lists open a centered dialog, like **Show votes**, with the message's title,
sections, option names, and descriptions.
Click an option or focus it and press Enter to send that selection. Close the
dialog or press Escape to leave without choosing.

Poll options show a result track even before votes arrive, and your selection
has a checkmark. **Show votes** opens participant names and vote times. If phone
history is still arriving, the dialog explains that earlier votes may be missing.
New polls received live start at zero without requesting earlier votes. Polls
received from history or while offline still recover results from the phone.
Polls archived by older builds may also need this recovery because those builds
did not retain whether the poll originally arrived live.

Carousel messages keep each card's image, text, and actions together. The timestamp
sits below the last card when the strip fits. When cards extend beyond the view,
round previous/next arrows appear over the strip. Click an arrow or focus it and
press Enter to move one card at a time. The arrows disappear at their respective
ends; **Shift + mouse wheel** and horizontal touchpad scrolling remain available
over the cards, without a bottom scrollbar. Image downloads follow your automatic-download
setting. Web links and copy-code buttons work; calls and unsupported carousel
reply actions remain unavailable.

| Carousel cards | Poll result details |
| --- | --- |
| ![Synthetic carousel with separate image cards and local actions](/screenshot-carousel.png) | ![Synthetic poll results showing participants and vote times](/screenshot-poll-results.png) |

| Grouped list dialog | Poll selection and results |
| --- | --- |
| ![Synthetic list dialog with section headings, descriptions and option selectors](/screenshot-interactive-list.png) | ![Synthetic poll with vote counts, result tracks and the selected answer](/screenshot-poll-voted.png) |

![Synthetic carousel cards in the light theme](/screenshot-carousel-light.png)

## Voice messages

Voice messages play in the chat with a seekable waveform. The chip beside the
waveform cycles the playback speed between 1x, 1.5x, and 2x. Right-click the
message for every speed, including 1.25x and 1.75x. The choice is remembered
for later messages. When a voice message ends, playback carries on through the
voice messages right after it that you have not heard yet, as on the phone;
any other message ends the run. The speaker's pitch stays the same at every
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
`Alt+↑/↓`, or `Ctrl+Shift+[` and `Ctrl+Shift+]` as in WhatsApp, to switch chats
without leaving the composer (Command instead of Ctrl on macOS). Within an open
chat, `PgUp`/`PgDn` scroll by about a page, and `Home`/`End` jump to the top or
the newest message (when the input is empty).

A shared contact message shows the name from its vCard. When the card names a
WhatsApp account, **Chat** opens a private conversation with it and, if the
person is not already in ZapFast's contacts, **Add** saves them, adding them to
your phone's contacts if you chose that for the last contact you added. A card with only a local number shows the number.

The chips under the search bar narrow the list to **Unread**, **Private**
(one-to-one chats), or **Groups**. A chip with unread chats shows how many it
has. Click the active chip again, or **All**, to see every chat. The
filter applies only to this list: search and the archive still show everything,
and it resets when ZapFast restarts.

Right-click a chat to pin, archive, or mute it for eight hours, one week, or
indefinitely. These changes also apply on your phone. Click the chat header to
see its picture, number, and group members.

## Labels

Labels are yours alone. They stay on this computer, they never reach your phone,
and nobody else sees them. They are not WhatsApp Business labels, and ZapFast
does not read or change those. Open **Labels** in any chat's right-click menu
and choose **Manage labels…** to make one, with a name and one of the offered
colours. ZapFast keeps up to twenty.

A chat can wear several labels at once. The **Labels** submenu of a chat's
right-click menu lists them, with a checkmark beside the ones the chat wears;
click one to add or remove it. Deleting a label takes it off every chat and
nothing else; the chats keep their messages.

Once a label exists, a row of label chips appears under the other chips, one
per label, with its colour and the number of unread chats wearing it, followed
by a **+** that opens the label manager. Pick a label to list only the chats
wearing it, channels included. A label is one more chip: picking it lets go of
**Unread** or **Groups**, and picking one of those lets go of the label. Like
the other chips, it does not narrow search or the archive. Click the active
label chip again, or **All**, to see every chat.

The button beside **New chat** (`Ctrl+B`) collapses the list to a narrow column
of avatars: unread chats show their badge, hovering names a chat, clicking opens
it, and `Ctrl+B` brings the full list back.

## Notifications and the tray

Closing the window keeps ZapFast linked in the tray. Click the tray icon or
launch the app again to reopen it. Launchers that support the Unity Launcher API
show the unread count as a badge on the app icon: KDE Plasma's taskbar, with
**Show badges** enabled in the Task Manager settings, and GNOME's Dash to Dock
or Dash to Panel. Windows overlays a compact unread-message count on ZapFast's
taskbar button while the window is open, using `99+` above 99. Windows must use
its regular taskbar icon size for overlays to appear. The count
does not count toasts remaining in Windows notification history. On Linux and
Windows, notifications show the chat picture and open the chat at the message
they announced when clicked. Muted chats do not send notifications, and
archived chats stay quiet until you unarchive them. You can change both settings.

Press `Ctrl+/` or click the keyboard button under the composer to list all
shortcuts.
