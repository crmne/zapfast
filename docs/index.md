---
layout: home
title: ZapFast
description: ZapFast is a lightweight WhatsApp client for Linux, macOS, and Windows, written in Rust with no browser engine. Opens in under a second, about 200 MB of RAM.
seo:
  title: Native WhatsApp Client for Linux, macOS and Windows
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
    dark: /screenshot.png
    light: /screenshot-light.png
    alt: "ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview"
    width: 1800
    height: 1360

features:
  - icon: ⚡
    title: Lightweight
    details: Opens in under a second and uses about 200MB of RAM. No browser engine.
  - icon: 🦀
    title: Native, in Rust
    details: "Built with [egui](https://www.egui.rs) and [fastframe](https://fastframe.dev), the framework behind a family of native desktop apps. No browser engine inside."
  - icon: 🎤
    title: Voice messages
    details: Play, seek, and record voice messages in the chat. OGG/Opus support is built in.
  - icon: 🖼️
    title: Attachments
    details: Photos, GIFs, stickers, documents, polls, locations, and link previews appear in the chat. Add captions before sending files.
  - icon: 🔔
    title: Background mode
    details: Closing the window keeps ZapFast linked in the tray. Notifications show the chat picture, and muted chats stay quiet.
  - icon: 👥
    title: Several numbers
    details: Link more than one WhatsApp number and switch between them from your picture at the foot of the rail beside the chat list.
  - icon: 🎨
    title: Themes
    details: Light, dark, or the system's appearance, custom colour themes, and on Omarchy your desktop theme.
    link: /themes/
    link_text: Make a theme
  - icon: ⌨️
    title: Keyboard shortcuts
    details: Search, switch chats, reply, and record with shortcuts. Select and copy text, including across messages.
  - icon: 🔓
    title: Open source
    details: MIT-licensed Rust that links through whatsapp-rust. The linking process is documented.
    link: https://github.com/crmne/zapfast
    link_text: Read the source
---

<script type="application/ld+json">
{"@context":"https://schema.org","@type":"SoftwareApplication","name":"ZapFast","url":"https://zapfast.rocks/","downloadUrl":"https://zapfast.rocks/download/","softwareVersion":"{{ site.zapfast_version }}","operatingSystem":"Linux, macOS, Windows","applicationCategory":"CommunicationApplication","license":"https://github.com/crmne/zapfast/blob/main/LICENSE","isAccessibleForFree":true,"offers":{"@type":"Offer","price":"0","priceCurrency":"USD"},"image":"https://zapfast.rocks/screenshot.png","author":{"@type":"Person","name":"Carmine Paolino","url":"https://paolino.me"}}
</script>

<p class="sibling"><strong>Want Spotify just as fast and native?</strong> <a href="https://spotifast.rocks">Spotifast</a> is ZapFast's sibling: the same native interface, for Spotify. Both are built on <a href="https://fastframe.dev">fastframe</a>, the shared foundation for native Rust apps built with <a href="https://www.egui.rs">egui</a>.</p>

<video class="hero-film image-src" width="1280" height="720" controls muted loop playsinline preload="metadata" poster="{{ '/assets/images/launch-film-poster.jpg' | relative_url }}" aria-label="ZapFast in under a minute: a film of the app, made with its built-in demo content" hidden>
  <source src="{{ '/assets/videos/launch-film.mp4' | relative_url }}" type="video/mp4">
</video>

<script>
  // The theme's hero takes a picture; the film takes its place where
  // scripts run, and the screenshot stays where they do not.
  (function () {
    var film = document.querySelector(".hero-film");
    var slot = document.querySelector(".VPHero .image-container");
    if (!film || !slot) return;
    slot.replaceChildren(film);
    film.hidden = false;
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      film.play().catch(function () {});
    }
  })();
</script>

<style>
  .hero-film {
    display: block;
    /* The film's box before its metadata arrives, so nothing moves. */
    aspect-ratio: 16 / 9;
    background: #0b141a;
  }
  .sibling {
    max-width: 1152px;
    margin: 48px auto 0;
    padding: 0 24px;
    text-align: center;
    color: var(--vp-c-text-2);
  }
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
