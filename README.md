# ZapFast

**WhatsApp, native and fast.** ZapFast is a WhatsApp client for Linux, macOS,
and Windows, written in Rust with [egui](https://github.com/emilk/egui) and
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust). It links to your
phone as a companion device and has no browser engine. In our Linux test it
opens in under a second and uses about 200 MB of idle RAM, against 1.13 GB for
WhatsApp Web. [See the measurements](https://zapfast.rocks/benchmarks/).

https://github.com/user-attachments/assets/2bf86b54-45fc-4add-8de7-c426c3cdad9b

<picture>
  <source media="(prefers-color-scheme: light)" srcset="docs/screenshot-light.png">
  <img src="docs/screenshot.png" alt="ZapFast showing a conversation with an attachment, voice messages, reactions, a quoted reply, and a link preview">
</picture>

**[zapfast.rocks](https://zapfast.rocks)** has downloads and the full guide:

- [What is ZapFast?](https://zapfast.rocks/what-is-zapfast/): what it does and does not do yet
- [Download](https://zapfast.rocks/download/): Linux, macOS, and Windows
- [Getting started](https://zapfast.rocks/getting-started/): linking your phone, building from source
- [Using ZapFast](https://zapfast.rocks/using-zapfast/): messages, stickers, polls, shortcuts
- [Settings & files](https://zapfast.rocks/settings-and-files/): where your data lives, encryption, app lock, updates
- [Making a theme](https://zapfast.rocks/themes/)

**Want Spotify just as fast and native?** [Spotifast](https://spotifast.rocks)
is ZapFast's sibling. Both are built on
[fastframe](https://github.com/crmne/fastframe).

## Sending through a rate limit

Messages go out in order. Only a typed WhatsApp rate-limit refusal, or waiting
behind its cooldown, shows **Waiting to send**. Waiting messages stay below new
replies and send automatically when the cooldown ends. **Cancel** deletes a
waiting message so it is never sent. A brief disconnect preserves that wait;
quitting or changing the proxy makes it **Not sent**. Ordinary offline sends
fail immediately, without joining the queue. An interrupted send shows **Send
unconfirmed**, because it may have reached WhatsApp, and is never resent
automatically. You can copy or delete it locally.

## Install

```sh
brew install --cask crmne/tap/zapfast   # macOS
yay -S zapfast-bin                      # Arch Linux
nix profile install github:crmne/zapfast
```

Installers, AppImages, and archives for every platform are on the
[releases page](https://github.com/crmne/zapfast/releases). To build from
source, see [Getting started](https://zapfast.rocks/getting-started/).

## Developing

```sh
cargo run --features demo -- --demo   # offline sample chats, no WhatsApp connection
```

The `queued-messages` and `queued-messages-stress` demo pages exercise this
queue over a synthetic offline link, including replies during cooldowns.

[CONTRIBUTING.md](CONTRIBUTING.md) covers issues, pull requests, and checks;
[AGENTS.md](AGENTS.md) the architecture; [DEMO.md](DEMO.md) demo pages,
screenshots, and recorded tours; [PACKAGING.md](PACKAGING.md) releases and
packages.

## Disclaimer

ZapFast is an unofficial client and is not affiliated with WhatsApp or
Meta. Using an unofficial client may be against WhatsApp's terms of service
and could get an account suspended. Use it at your own risk.

## License

MIT. Inter and Noto Color Emoji are under the SIL Open Font License; the icons
and the chat wallpaper doodles are from [Lucide](https://lucide.dev) (ISC).
The notification sounds are [Pidgin](https://pidgin.im)'s, under the GPL-2.0
(see `assets/sounds/`).
