---
layout: home
title: ZapFast
description: A fast, lightweight WhatsApp app for Linux, macOS, and Windows.
permalink: /
hero:
  name: ZapFast
  text: WhatsApp, native and fast
  tagline: A lightweight WhatsApp app for Linux, macOS, and Windows. Chat, send voice messages, and share files.
  actions:
    - theme: brand
      text: Download
      link: /download/
    - theme: alt
      text: What is ZapFast?
      link: /what-is-zapfast/
    - theme: alt
      text: GitHub
      link: https://github.com/crmne/zapfast
  image:
    src: /screenshot.png
    alt: "ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview"
    width: 1387
    height: 1040

features:
  - icon: ⚡
    title: Lightweight
    details: Opens in under a second and uses 150MB of RAM. No browser engine.
  - icon: 🎤
    title: Voice messages
    details: Play, seek, and record voice messages in the chat. OGG/Opus support is built in.
  - icon: 🖼️
    title: Attachments
    details: Photos, GIFs, stickers, documents, polls, locations, and link previews appear in the chat. Add captions before sending files.
  - icon: 🔔
    title: Background mode
    details: Closing the window keeps ZapFast linked in the tray. Notifications show the chat picture, and muted chats stay quiet.
  - icon: ⌨️
    title: Keyboard shortcuts
    details: Search, switch chats, reply, and record with shortcuts. Select and copy text, including across messages.
  - icon: 🔓
    title: Open source
    details: MIT-licensed Rust built with egui and whatsapp-rust. The linking process is documented.
    link: https://github.com/crmne/zapfast
    link_text: Read the source
---

<style>
  /* Override the square hero slot to fit the screenshot. */
  .VPHero .image-container {
    width: 100% !important;
    height: auto !important;
    transform: none !important;
  }
  .VPHero .image-src {
    position: relative !important;
    top: auto !important;
    left: auto !important;
    transform: none !important;
    width: 100% !important;
    height: auto !important;
    max-width: 100% !important;
    max-height: none !important;
    padding: 0 !important;
    border-radius: 12px;
    box-shadow: 0 12px 48px rgba(0, 0, 0, 0.45);
  }
  @media (max-width: 959px) {
    .VPHero .image {
      margin: 0 0 24px !important;
    }
  }
</style>
