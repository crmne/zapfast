# Developing with the demo

The `demo` feature builds ZapFast with offline sample chats, for development,
screenshots, and recorded tours. `AGENTS.md` describes the architecture and
the rules for changes; `CONTRIBUTING.md` lists the checks.

```sh
cargo run --features demo -- --demo            # sample chats, no connection
cargo run --features demo -- --demo-page login # or settings, pair, info, light, …
cargo run --features demo -- --demo-shot shot.png --demo-page chat,light
cargo run --features demo -- --demo-tour      # Space starts/replays a 41-second tour
cargo run --features demo -- --demo-tour --demo-tour-script whats-new # what 0.16 added
cargo run --features demo -- --demo-hover 900,400 # holds a fake pointer there
cargo test --all-features                      # includes a headless layout of every screen
cargo clippy --all-targets --all-features -- -D warnings
```

To include a default GIPHY key for GIF search, set it at build time. A key in
Settings overrides it:

```sh
ZAPFAST_GIPHY_KEY=your-key cargo build --release
```

The earlier `FASTSAPP_GIPHY_KEY` build variable remains supported as a fallback.

CI checks the complete lockfile against RustSec advisories with `cargo audit`.
Candidate-specific manual checks and results are tracked in the release PR.

## Recording a demo

The `demo` feature uses offline sample chats in a fresh temporary directory.
It does not open your linked account, read your message archive, connect to
WhatsApp, or register a tray icon. You can run it alongside your regular app.

```sh
cargo build --locked --features demo
./target/debug/zapfast --demo-tour --demo-size 1280x800
```

The **ZapFast Demo** window waits for **Space**. The 41-second tour starts with
search, switches chats with keyboard shortcuts, scrolls, right-clicks a message
and selects Reply, types quickly, completes emoji and mentions, searches the GIF
picker and sends a still sticker, opens group information and the shortcut list,
and changes themes through Settings. It uses the normal mouse and keyboard handlers;
a local responder handles outgoing messages with no WhatsApp connection.
The demo's profile pictures, photos, video clips, GIF-search thumbnails, and captioned stickers
are stock media compiled into demo builds only, all CC0 or in the public domain
(`assets/demo/SOURCES.md` lists each source); demo GIF search uses these local
fixtures, and the remaining stickers come from the bundled Noto emoji font. The tour makes no
sound and holds its final frame. Space rebuilds the sample and replays.
For an automatic start, add `--demo-tour-delay 5000` (milliseconds).
Use `--demo` instead of `--demo-tour` to explore the sample chats yourself.

`--demo-tour-script whats-new` plays an 86-second tour of what ZapFast 0.16
added instead: the composer's plus menu and poll dialog, searching a chat and
narrowing it to a day, the photo preview, videos and round video messages
playing in place, sticker shelves and sticker search, message info in a group,
the Favorites and label chips and a chat's menu, recording a voice message and
choosing a playback speed, the chat list folded to avatars, hover controls,
Ctrl-click and Shift-click selection with Forward, and Settings (languages,
search, and the light theme). `--demo-tour-script launch` is the default.
Demo runs never open the microphone: recording plays back a synthetic tone.
Use `--demo-page rtl-self` for a self-chat of mixed Hebrew, Arabic, and
English lines.
Use `--demo-page composer-tools` to preview the WhatsApp-style composer pill
and its attachment and poll menu. `typing`, `mention`, and
`emoji-complete` preview the multiline field and inline suggestions.
Use `--demo-page chat-menu` to preview the compact chat context menu,
`--demo-page chat-header-menu` for the menu at the top of an open chat, and
`--demo-page chat,voice,voice-menu` for a voice message's menu with its speeds.
`--demo-page motion` shows a motion photo, `motion-playing` plays its clip in
the bubble, and `motion-preview` opens its preview.
`--demo-page video` shows a video and round video messages,
`--demo-page video-expanded` the video over the whole window, and
`video-playing` or `note-playing` starts one of them, silently.
Locked chats preview with `channel`, `locked`, `locked-open` (the code is
`demo-code`), `locked-prompt`, `locked-setup`, and `keyring`; the app lock
with `app-lock` (the password is `demo-password`), `app-lock-wrong`,
`app-lock-forgot`, `app-lock-settings`, and `app-lock-setup`. `new-chat`,
`unnamed-group`, and `react-picker` show those dialogs; `group-info`,
`group-info-rename`, `group-info-saving`, and `group-info-locked` a group's
editable info; `meta-ai` a Meta AI reply with code and a table.
For deterministic theme screenshots, `--demo-page settings,omarchy` and
`--demo-page settings,omarchy-light` preview following dark and light Omarchy
palettes without changing the desktop theme.

Use `--demo-page preview` for a photo viewer with three pictures, text between
the second and third picture. Hover within the photo's vertical bounds,
including the side gutters, to show the previous/next arrows at fixed positions
outside the photo, or use Left/Right arrow keys. Use `--demo-hover 700,450`
to include the arrows in a screenshot.
Use `--demo-page shared-contact` for an offline shared-contact card with synthetic
vCard data, or `--demo-page interactive` for text and button messages, or
`--demo-page interactive-media` for messages with an image, and
`--demo-page interactive-list` for a list message,
`--demo-page interactive-list-dialog` for its grouped choice dialog, `--demo-page carousel`
for a scrolling strip or `--demo-page carousel-pair` for two cards, and `--demo-page poll-empty`, `poll-voted`, or `poll-results`
for voting states. Use `--demo-page interactive-actions` for reply, list, copy-code, and unavailable
form actions. Add `,light` to
preview any of these in the light theme. Capture the app's own frame without desktop
content:

```sh
./target/debug/zapfast --demo --demo-page interactive-media --demo-shot interactive.png
./target/debug/zapfast --demo --demo-page interactive-media,light --demo-shot interactive-light.png
```

On Omarchy, run `omarchy screenrecord`, select the demo window, then press Space
in ZapFast. Recording has no audio unless you explicitly enable desktop or
microphone audio. Stop with `omarchy screenrecord --stop-recording` after the
tour finishes. The default capture records a fixed rectangle, so keep the demo
window visible and stationary until recording stops.

To annotate the video with a visible pointer, click rings, and outlined shortcut
labels, add `--demo-tour-events tour.json` when launching the tour. After
recording, run:

```sh
python3 scripts/render-demo.py recording.mp4 tour.json launch.mp4 --start 0.8
```

Set `--start` to the recording time (in seconds) when you pressed Space. The
export trims the setup footage, adds a caption band below the app, and produces
a silent H.264 MP4. It requires `ffmpeg` with libass support and `ffprobe`.
`--scale 1.5` keeps 1.5 pixels per point, for example 1920 pixels across from a
1280-point window recorded at 2x; the default is one pixel per point.

These annotations are added during video export, not drawn by the app. The
trace contains only pointer coordinates and shortcut labels, not typed text.

Instead of recording the screen, the tour can save its own frames. With
`--demo-tour-frames DIR`, it starts at once, plays on a virtual clock (steady
frame times even when a frame is slow to draw), writes every frame as a PNG
at the window's pixel size, and quits when the tour ends. `--demo-fps` sets the
rate (30 by default). The window still has to be shown somewhere; a virtual
output keeps it off your screens. Then assemble and annotate the frames:

```sh
cargo build --release --locked --features demo
./target/release/zapfast --demo-tour --demo-tour-script whats-new \
  --demo-size 1280x800 --demo-tour-frames frames --demo-tour-events tour.json
ffmpeg -framerate 30 -i frames/frame-%05d.png -c:v libx264 -crf 12 -pix_fmt yuv420p raw.mp4
python3 scripts/render-demo.py raw.mp4 tour.json whats-new.mp4 --scale 1.5
```
