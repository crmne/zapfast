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

WhatsApp answers a limited number of QR codes on one connection and then ends
it. ZapFast opens a fresh connection for a new set, so the linking screen keeps
showing a current code until you scan it.

Each linked device can have one connection. Two clients using the same device
keys will disconnect each other. A second ZapFast launch reopens the existing
window instead of starting another instance.

**Development builds:** when WhatsApp identifies a contact previously known
by a private id, ZapFast merges that chat into the contact's phone-number
chat. Messages keep their stars, pins, and motion-photo clips. Newer star and pin
changes take precedence, including removing a star or pin.

## What the phone is for

- **History.** WhatsApp replays only recent history to a fresh device.
  ZapFast archives new messages and asks the phone for older messages when
  you scroll past the archive, or in the background when **Fetch older
  history** is on in Settings > Chats. The phone must be online for these
  requests and to re-upload expired attachments.
- **Other features.** Sending, receiving, receipts, and presence go through
  WhatsApp's servers directly; the phone can be offline for all of it.

## What stays local

The message archive, media, and session keys stay on your computer.
[Settings & Files](/settings-and-files/) lists their paths. ZapFast connects
to WhatsApp's servers, GIPHY for the GIF picker and search, and Signal's sticker
CDN when you import a Signal sticker pack. It has no telemetry. It also asks
`api.github.com` once a day whether a newer release exists; you can turn this
off in Settings. Downloading an update fetches release files from GitHub.

Unlinking from Settings tells the phone to forget the device and deletes
the local archive and caches.
