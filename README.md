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

New chat can check an international phone number with WhatsApp and open a
conversation without saving a contact. See [Chats](docs/_guide/using-zapfast.md#chats).

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
