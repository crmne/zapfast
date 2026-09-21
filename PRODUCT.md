# Product

## Register

product

## Users and purpose

ZapFast serves people reading and sending WhatsApp messages on Linux, macOS,
and Windows. It is a small native companion client built with Rust and egui.
The conversation is the primary workspace, with chats beside it and a composer
below it.

## Product character

Native, familiar, restrained. Preserve the existing interface, fonts, local
themes, and platform behaviour. WhatsApp Web is the supplied reference for
interactive message anatomy: media, readable text, message metadata, then
separated action rows.

## Design principles

- Keep messages readable and the composer immediately available.
- Use existing shared controls and theme colours consistently.
- Make working actions and unavailable actions distinguishable.
- Preserve selection, copying, emoji, links, and narrow-window usability.
- Validate screenshots with synthetic offline content only.

## Boundaries and accessibility

No browser engine, hosted backend, telemetry, or additional account system.
Avoid decorative dashboards and extra nested cards inside message bubbles.
Retain keyboard operation, zoom, text selection, and light and dark themes.
No additional accessibility certification target is specified by the project.
