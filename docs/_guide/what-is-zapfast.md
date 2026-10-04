---
title: What is ZapFast?
description: Why ZapFast exists, what it supports, and its current limitations.
redirect_from:
  - /what-is-fastsapp/
nav_order: 0
---

## Why ZapFast

WhatsApp has no official Linux app. ZapFast is a native WhatsApp client
written in Rust with [egui](https://github.com/emilk/egui). It connects through
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust). ZapFast is a
single binary with no browser engine and uses a layout similar to WhatsApp Web.
In our Linux test, it opens in under a second and uses about 200 MB of idle
RAM, compared with 1.13 GB for WhatsApp Web and its Chromium processes.
[See the measurements](/benchmarks/).

<img class="VPImage dark" src="{{ '/screenshot.png' | relative_url }}" alt="ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview" width="1800" height="1360">
<img class="VPImage light" src="{{ '/screenshot-light.png' | relative_url }}" alt="ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview" width="1800" height="1360">

## What it does

- **Stores your chats.** ZapFast links as a companion device and stores
  messages in an encrypted SQLite file per account. History remains after
  restart, and older messages are fetched from your phone as you scroll.
- **Sends common message types.** Send formatted text, replies, edits,
  reactions, forwards, pictures, files, stickers, GIFs, and recorded voice
  messages.
  You can add captions to attachments before sending them.
- **Plays media in the chat.** Voice messages, videos, round video messages,
  GIFs, and animated stickers play in place. H.264 videos in MP4 files play
  after downloading to the local cache; other formats open in your system
  player. [Download limits and playback controls](/using-zapfast/#videos-and-photos).
- **Uses interactive messages.** Business templates show their text, images,
  and options. Reply buttons and simple lists send the selected response with
  a quote, web links open in your browser, and copy-code buttons use the clipboard. [See examples and limitations](/using-zapfast/#interactive-messages).
- **Uses consistent names.** Address-book names take priority over public
  WhatsApp profile names for chats, mentions, replies, and notifications.
- **Runs in the background.** Closing the window keeps ZapFast in the system
  tray. Notifications can show the chat picture and open the chat at the
  message they announced. Supported desktops show the unread count on the app
  icon in the taskbar or dock; on Windows, the count appears while the window
  has a taskbar button. Muting a chat also mutes it on your phone.
- **Keeps several numbers.** Link more than one WhatsApp number and switch
  between them in one window.
- **Keeps chats private on this computer.** The archive is encrypted with a
  key in your OS keyring, locked chats stay hidden behind a local code, and an
  optional app lock hides the whole window behind a password.
- **Copies message text.** Select part of a message or copy across messages
  with the time, date, and sender included.

## What it does not do yet

These are current implementation limits, not permanent product exclusions.
Feature requests can be discussed within the project's
[product boundaries](https://github.com/crmne/zapfast/blob/main/CONTRIBUTING.md#before-opening-an-issue).
ZapFast does not currently support:

- Calls, status posts, communities, publishing to channels, and group
  administration beyond a group's name and photo (members, admins,
  descriptions, settings).
- Playing videos outside the supported H.264 MP4 format in the app; they open
  in your system player. Downloads over 64 MiB are currently blocked for all
  attachment types, including videos, even when clicked manually.
- Interactive forms, payments, shopping flows, carousel selections, or forwarding
  interactive messages. Use these in WhatsApp Web or on your phone. Embedded
  videos, documents, and templates without readable text
  also need another client.

In **0.19.0**, **Delete for me** for individual messages affects only the local
copy. Syncing these deletions with the phone and other linked devices is now
implemented on main, but has not yet been released. See
[deletion behavior](/using-zapfast/#writing).

When reporting [an issue](https://github.com/crmne/zapfast/issues), include
what happened, what you expected, and when it happened. This helps match the
problem to the log in the state directory.

## Account safety

ZapFast is an **unofficial** client. Using it may be against WhatsApp's terms
of service. It uses WhatsApp's companion-device protocol, sends normal
receipts, and does not automate or send messages in bulk. WhatsApp may still
restrict accounts that use unofficial clients. Use an official client if you
cannot accept that risk.

## Prior art

ZapFast connects through
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust), which grew out
of the [whatsmeow](https://github.com/tulir/whatsmeow) lineage. WhatsApp
Web defines the companion-device model. ZapFast is a sibling of
[Spotifast](https://spotifast.rocks), a native client for Spotify. Both are
built on [fastframe](https://github.com/crmne/fastframe), the shared foundation
for native Rust apps built with egui.

ZapFast is an independent project, not affiliated with or endorsed by
WhatsApp LLC or Meta. WhatsApp is a trademark of WhatsApp LLC.
