---
title: Custom emoji fonts
description: Embed your own colour emoji font when building ZapFast from source.
nav_order: 11
---

ZapFast normally uses your desktop's emoji artwork, with bundled Noto Color
Emoji for missing pictures. A source build can embed a different font by
setting `ZAPFAST_EMOJI_FONT` to a local font file before running Cargo.

On Windows, in PowerShell:

```powershell
$env:ZAPFAST_EMOJI_FONT = 'C:\Fonts\MyEmoji.ttf'
cargo build --locked --release
```

On Linux or macOS:

```sh
ZAPFAST_EMOJI_FONT=/path/to/MyEmoji.ttf cargo build --locked --release
```

The renderer supports colour bitmap fonts using CBDT/CBLC or sbix tables,
including supported TrueType/OpenType collections (the first face is used).
A colour outline font alone is not sufficient. The file must cover the
emoji and joined sequences you need. The custom build does not fall back
to the system emoji font or Noto; unsupported sequences retain the
renderer's existing partial-sequence or text fallback.

The build script copies the file into Cargo's output directory and embeds
it in the executable. No runtime file access, font installation, or network
download is needed. Messages, reactions, picker cells, and text fields use
the same renderer; emoji sent to other people remain standard Unicode.
The offline tour's generated sticker fixtures still use Noto.

Cargo watches the input font and the environment variable for changes.
To return to the normal build, unset the variable and rebuild:

```powershell
Remove-Item Env:ZAPFAST_EMOJI_FONT
cargo build --locked --release
```

```sh
unset ZAPFAST_EMOJI_FONT
cargo build --locked --release
```

Supply only fonts you have permission to use. A custom font keeps its own
license, and embedding it in an executable does not grant redistribution
rights. ZapFast does not download or distribute Apple emoji fonts.
