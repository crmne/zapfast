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
In our Linux test, it opened in under a second and used about 150 MB of idle
RAM, compared with 1.13 GB for WhatsApp Web and its Chromium processes.
[See the measurements](/benchmarks/).

![ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview](/screenshot.png)

## What it does

- **Stores your chats.** ZapFast links as a companion device and stores
  messages in one SQLite file. History remains after restart, and older
  messages are fetched from your phone as you scroll.
- **Sends common message types.** Send formatted text, replies, edits,
  reactions, forwards, pictures, files, stickers, GIFs, and recorded voice
  messages.
  You can add captions to attachments before sending them.
- **Plays media in the chat.** Voice messages, GIFs, and animated stickers
  play in place. The required audio and video decoders are built in.
- **Uses consistent names.** Choose address-book names or public WhatsApp
  profile names for chats, mentions, replies, and notifications.
- **Runs in the background.** Closing the window keeps ZapFast in the system
  tray. Notifications can show the chat picture and open the chat. Muting a
  chat also mutes it on your phone.
- **Copies message text.** Select part of a message or copy across messages
  with the time, date, and sender included.

## What it does not do yet

ZapFast does not currently support:

- Calls, status posts, communities, newsletters, and group administration.
- Playing ordinary videos in the app; they open in your player. Voice
  messages and GIFs do play in place.
- Replying with an attachment (replying with text or a voice message
  works).
- Colour emoji on Windows: Segoe UI Emoji is not a bitmap font, so emoji
  stay monochrome there for now.

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
[Spotifast](https://spotifast.rocks), a native client for Spotify.

ZapFast is an independent project, not affiliated with or endorsed by
WhatsApp LLC or Meta. WhatsApp is a trademark of WhatsApp LLC.
