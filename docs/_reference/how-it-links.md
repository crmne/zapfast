---
title: How it links
description: How ZapFast links, when it needs the phone, and what it stores locally.
nav_order: 1
---

## A companion device

ZapFast registers as a linked device through
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust). Messages are
end-to-end encrypted for each device. Pair with a QR code or phone-number
code. New links list ZapFast under **Linked devices**; an existing link may
retain the earlier name FastsApp until you link again. There you can unlink
it at any time.

Each linked device can have one connection. Two clients using the same device
keys will disconnect each other. A second ZapFast launch reopens the existing
window instead of starting another instance.

## What the phone is for

- **History.** WhatsApp replays only recent history to a fresh device.
  ZapFast archives new messages and asks the phone for older messages when
  you scroll past the archive. The phone must be online for these requests and
  to re-upload expired attachments.
- **Other features.** Sending, receiving, receipts, and presence go through
  WhatsApp's servers directly; the phone can be offline for all of it.

## What stays local

The message archive, media, and session keys stay on your computer.
[Settings & Files](/settings-and-files/) lists their paths. ZapFast connects
to WhatsApp's servers and, when you search for GIFs, GIPHY. It has no
telemetry. It also asks `api.github.com` once a day whether a newer release
exists; you can turn this off in Settings.

A link a sender left without a preview makes ZapFast read that page once, from
this computer, so the message can show a card as the phone app shows it. Only
the address the sender wrote is requested, and never the conversation, the
contact, or the message. **Settings > Chats > Preview links** turns this off.

Unlinking from Settings tells the phone to forget the device and deletes
the local archive and caches.
