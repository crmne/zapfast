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

### Passkey verification

Development builds can answer WhatsApp's additional passkey challenge through
a nearby phone. This is separate from the WhatsApp linking QR code. Select
**Start**, then scan the passkey QR code with the phone's camera, not the
Linked devices scanner. Approve the request using a WhatsApp passkey already
available on that phone. ZapFast does not need access to your password manager
or the passkey's private key.

Both devices need Bluetooth and internet access. Linux uses BlueZ through
system D-Bus; macOS needs Bluetooth permission. On supported Windows 11
versions, Windows displays the authentication dialog and its QR code.
The phone's operating system determines which passkey providers it offers.

For a fresh link, compare the code shown on both devices before continuing.
Cancel and failed-attempt retries keep the local archive and session files.
The authenticator request expires after at most one minute; the remaining
linking steps have a two-minute overall deadline after **Start**.
Failed attempts retain their original error instead of replacing it when that
deadline passes. Diagnostic logs record linking stages and error categories,
not QR payloads, challenges, or credentials.
Optional browser metadata, transport hints, and unsupported WebAuthn extensions
do not block the QR flow. The server's challenge, relying party, allowed
credential IDs, and user-verification requirement stay intact.

This integration is experimental. Its hybrid transport library warns that
the implementation is incomplete and has not had a cryptographic security
review. Live compatibility with a particular phone or passkey provider must
be checked before relying on it.

The published `webauthn-authenticator-rs` 0.6.2 crate drops the authenticator's
user handle from CTAP assertions on Linux and macOS. Linking can fail when
WhatsApp requires that value. The successful local trial used a dependency
patch; this build uses the unmodified published crate and has not had the
same end-to-end verification. Windows uses the native WebAuthn API instead.

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
CDN when you import a Signal sticker pack. While you write a link in a new text
message, it also contacts that link's site, and whichever site the page names
for its picture, to build a [preview](/using-zapfast/#writing). It has no
telemetry. It also asks
`api.github.com` once a day whether a newer release exists; you can turn this
off in Settings. Downloading an update fetches release files from GitHub.

Passkey verification also uses Bluetooth for proximity and an encrypted
hybrid-authentication tunnel through infrastructure operated by Apple or
Google. The tunnel carries authentication traffic, not chats or contacts.

Unlinking from Settings tells the phone to forget the device and deletes
the local archive and caches.
