---
title: Making a theme
description: Write a JSON palette to colour ZapFast your way.
nav_order: 5
---

## Where themes live

A theme is a small JSON file in the `themes` folder beside `settings.json`:

| Platform | Themes folder |
| --- | --- |
| Linux | `~/.config/zapfast/themes/` |
| macOS | `~/Library/Application Support/me.paolino.zapfast/themes/` |
| Windows | `%APPDATA%\paolino\zapfast\config\themes\` |

The quickest way there is **Settings → Appearance → Open themes folder**,
which creates the folder if needed. Each file shows up in the Theme picker
under its name: `gruvbox.json` becomes **Gruvbox**.

The built-in palettes (Catppuccin, Nord, Rose Pine, and the others in the
picker) are there too, as ordinary files written the first time ZapFast
starts: read them, change them, or start from one. They are yours after that:
ZapFast never rewrites them, and one you delete stays deleted.

## A first theme

Save this as `themes/gruvbox.json`:

```json
{
  "base": "dark",
  "colors": {
    "window": "#282828",
    "panel": "#1d2021",
    "surface": "#32302f",
    "text": "#ebdbb2",
    "accent": "#b8bb26"
  }
}
```

`base` is `dark` (the default) or `light`. Every colour you leave out comes
from that base, so a theme can be as short as one accent colour. Colours are
`#RRGGBB`, or `#RRGGBBAA` with transparency.

## Colours

| Name | What it colours |
| --- | --- |
| `window` | The window's background |
| `panel` | The chat list and Settings |
| `surface` | Fields, cards, and menus |
| `surface_hover` | A surface under the pointer |
| `surface_active` | The selected chat |
| `outline` | Borders and dividers |
| `text` | Main text |
| `secondary` | Less important text, such as previews and times |
| `dim` | Hints and placeholders |
| `accent` | Buttons, selection, and highlights |
| `accent_hover` | An accent control under the pointer |
| `on_accent` | Text and icons on the accent colour |
| `danger` | Destructive actions and errors |
| `warning` | Warnings |
| `overlay` | Dialogs and floating cards |
| `shadow` | Shadows |
| `chat` | The conversation background, and the **Theme** wallpaper |
| `bubble_in` | Messages from others |
| `bubble_out` | Your messages |
| `link` | Links in messages |
| `read` | The ticks of read messages |
| `focus` | The outline around focused controls and the field you are typing in |

When a theme sets some colours but not the chat ones, ZapFast works them out:
`chat` follows `window`, `bubble_in` follows `surface`, `bubble_out` is
`surface` tinted with `accent`, `link`, `read` and `focus` follow `accent`, and
`overlay` follows `panel`. That is why Spotifast palettes work as they are.

## Trying it out

On Linux, ZapFast notices a new or changed file by itself. On macOS and
Windows, run `zapfast reload-themes` after saving; it refreshes the list and
the selected theme without restarting, and never opens a stopped app.

A file with a mistake is skipped, and its reason is in the log. ZapFast keeps
the last palette that worked, so a broken or deleted file never resets your
appearance.

On Omarchy, **Follow system** already uses your current Omarchy colours, and
ZapFast follows each theme change as it happens.
Elsewhere on Linux it takes the light or dark preference your desktop
publishes through its settings portal (GNOME, KDE, and Flatpak), and
switches as soon as you change it.
