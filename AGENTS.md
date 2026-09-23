# ZapFast agent guide

ZapFast is a small native WhatsApp client: Rust, egui, and the
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) library for the
protocol. These notes are for coding agents and new contributors.

## Product boundaries

- Keep it a small native client. No browser engine, no telemetry, no
  hosted backend, no ZapFast-operated account system. Features never send
  message content to a third party.
- Do not vendor, fork, or patch upstream crates (egui, epaint, whatsapp-rust)
  in this repository. Fix them upstream.
- The protocol comes from whatsapp-rust. Do not reimplement pieces of it
  here, and do not advertise a capability merely because a protobuf field
  for it exists.
- Do not broaden a task into adjacent features or a general refactor.
  Preserve existing user behaviour unless the task changes it.

## Privacy

- The user's archive is personal data. Do not read chat rows, message
  bodies, contacts, or other user content out of `archive.db` or any
  exported log, not even read-only. Schema, column existence, and row
  counts are fine; message contents are not.
- When a bug report or feature needs the user's data, hand the user the
  query or command to run and let them report the result back.
- Never log message contents, phone numbers, keys, or QR payloads at a
  level that ships (see the definition of done); treat existing
  captures of them the same way.

## Architecture

- `src/ui/` draws views and pushes `model::Action`s; `src/app.rs` applies
  them after the frame. Never mutate application state from inside a view
  beyond the view's own fields (composer text, search text, flags).
- `src/backend.rs` is the interface's handle to a tokio runtime on its own
  thread; `src/backend/worker.rs` runs there. It owns the whatsapp-rust
  `Bot`, the message archive, downloads, and profile pictures. The two
  sides talk only through `Command` (interface to runtime) and `Event`
  (runtime to interface); every event wakes the window through `Waker`.
- `src/archive.rs` is the SQLite store of chats, messages, contacts, and
  privacy-id mappings. WhatsApp replays history once, at link time, so the
  archive is the only copy. It keeps each message's raw protobuf because
  the keys to fetch an attachment live in it. `src/archive/encryption.rs` opens
  the archive with SQLCipher and a random key stored in the OS keyring. Plaintext
  migration checkpoints the old WAL and verifies an encrypted staging file before
  atomic replacement. A locked or missing key stops linking; never fall back to
  a disposable archive. Tests use fixtures and mock credentials only.
- `src/model.rs` holds the app's own types. Views never touch a protobuf;
  the worker translates in `classify()` and `parse_conversation()`.
- Interactive messages are parsed in `backend/worker/interactive.rs`. Views receive
  labels and local capabilities, never protocol option ids. `ReplyInteractive`
  carries only the archived message id and visible button/choice indices;
  `interactive/replies.rs` re-resolves them from raw protobuf and uses the library's
  quote context and normal send path. Only known quick replies and single-select
  lists may send responses. Copy-code actions stay local. Do not turn arbitrary
  flow JSON into replies or fall back to sending its visible label as plain text.
  The versioned archive backfill must preserve downloaded image paths and edits.
  Carousel cards retain independent images and local actions. Download commands
  carry an optional card index, and the archive stores each image path separately.
  The version-4 backfill preserves those paths when rebuilding derived content.
  See [compatibility notes](docs/interactive-message-actions.md) for response
  families, source references, and live-test limits.
- Poll creation, voting, and decryption use whatsapp-rust's `Client::polls()`.
  `backend/worker/polls.rs` retains the original creator identity and key in the
  encrypted archive; `archive/polls.rs` keeps each voter's latest timestamp and
  message id, including encrypted updates whose parent has not arrived yet.
  History replay must not undo a newer vote or withdrawal. Decryption runs in
  batches of eight, with failures retried after reconnecting. The interface receives
  option counts, its own selection, and the latest decrypted voter names/times
  for the results dialog, never keys or protobufs. New, non-offline poll creations
  received through normal delivery start with a complete zero-vote baseline,
  persisted in `poll_history`. PDO recovery and
  duplicate deliveries do not establish that baseline. Visible polls without a
  complete baseline request phone history automatically, anchored after the
  creation message so the response includes its vote snapshot. `poll_history.rs` serializes these
  requests and retries from 30 seconds to 15 minutes without an interface timer.
  History request timestamps are Unix seconds: the library argument and wire
  field misleadingly end in `Ms`. Do not multiply archive timestamps by 1,000.
  A repeated poll question with no usable vote snapshot cannot finish recovery.
- Chat ids are canonical strings: a chat behind a privacy id (`@lid`) is
  filed under its phone number once the mapping is known. Use
  `Worker::canonical` for anything that arrives as a `Jid`.
- `src/updates/` downloads verified GitHub releases and hands installation to a
  helper after an explicit restart action. Keep package-manager detection, asset
  checksums, startup acknowledgement and rollback intact. Portable releases carry
  `packaging/zapfast-portable.txt`; the Windows installer has its own marker.
- `src/theme/custom.rs` scans local JSON palettes off the UI thread, caching the
  last usable choice in settings, with shared Spotifast palettes embedded as
  defaults. On Linux filesystem notifications reload the catalog and the active
  Omarchy palette without a repaint timer; following Omarchy does not require
  packaged assets. Native packages ship optional hooks and templates, preserving
  existing per-user files. `reload-themes` uses the single-instance channel
  without opening a window.
- `src/theme.rs` owns colours, fonts, and icons; `src/ui/widgets.rs` the
  shared controls. New icons go in `assets/icons/` as 24px Lucide-style SVGs
  and in the `icons!` table.
- `src/markup.rs` turns WhatsApp's text markup, links, and mentions into an
  egui `LayoutJob`; `src/emoji.rs` swaps every emoji for a placeholder
  glyph at layout time and paints the desktop's colour emoji bitmap over
  it afterwards (resolving sequences through the font's GSUB ligatures).
  Any text that can hold an emoji goes through `widgets::line` /
  `widgets::rich_text` or `markup::layout`, never a bare `Label`.
- `src/animation.rs` plays animated stickers and GIFs: WebP/GIF frames
  decode in-process, and so do MP4s (the `mp4` crate demuxes, `openh264`
  decodes the H.264 WhatsApp uses, samples converted from AVCC to Annex
  B); `ffmpeg` is only a fallback for other codecs. `openh264` compiles
  its C++ from source with the C++ compiler of the host; `nasm` is
  optional and only adds the SIMD paths (the AUR recipes leave it out,
  the build works without it). Frames become textures on the interface
  thread and are dropped when unseen.
- `src/video.rs` plays other videos inside their message, one at a time,
  with the same `mp4` and `openh264` pieces: a thread decodes from the
  keyframe before the start (openh264 must not flush after each packet or
  B-frames stop it) and streams scaled frames with presentation times; the
  interface thread shows the due frame in one texture. rodio's symphonia
  decodes the AAC track and its position steers the clock. Non-H.264 files go
  to the system player. `Action::PlayVideo/SeekVideo/ToggleVideoSound` drive
  it; leaving the chat stops it and an unseen video pauses. Round video
  messages (PTV) are `Content::Video { note: true }` and draw as circles.
- Message bodies paint through `markup::paint_selectable` and single lines
  through `widgets::selectable_rich_text`: both hand the galley to
  `egui::text_selection::LabelSelectionState` (which paints it) and only
  overlay the colour emoji, so text can be swept and copied while
  `style.interaction.selectable_labels` stays false for every other label.
  The response must sense clicks and drags. `SelectionLeash` (an egui
  `input_hook` plugin) clamps a drag that started in the message view to
  just inside its edge once the pointer strays out (the platform keeps
  reporting a grabbed pointer beyond the window), and drops mid-drag
  `PointerGone`, so the selection keeps a row under it while the edge
  scroll brings more past. A copy that sweeps across
  messages is rebuilt by `src/transcript.rs` with `[time, date] Name:`
  per message (the phone's sharing format): every drawn body lands in
  `App::copy_rows` each frame, and the `CopyAnnotator` egui plugin
  rewrites the queued `CopyText` in `output_hook`, the only hook that
  runs after the selection plugin's own end-of-pass flush (plugins run
  in registration order and the built-ins come first, so end-pass
  callbacks fire too early).
  Selection galleys share the message viewport's horizontal bounds while
  retaining their glyph positions: otherwise egui considers short incoming
  and outgoing messages separate columns and will not sweep across them.
- Group names and members come from `groups().get_metadata`, asked one
  turn at a time (two per 5 s tick, `pump_group_info`): dozens of unnamed
  groups arrive with history sync and a burst of queries hits the
  server's rate limit, which once left groups called "Group" forever.
  Failures back off (30 s doubling, seven tries); item-not-found,
  forbidden and not-authorized are final and stop the asking.
- A download that answers 403/404/410 goes through
  `client.media_reupload().request(..)` (a server-error receipt; WhatsApp
  has the phone re-upload and answers with a fresh `direct_path`) and is
  fetched once more before the bubble reports "No longer on WhatsApp's
  servers". Download failures never toast; they live in the bubble as
  "... · click to retry". Copied text is refined by
  `transcript::refine`: emoji placeholders map back through each row's
  `placements`.
- History sync can bring a chat with a name and no messages at all; a
  history request for such a chat is anchored at the present with an
  empty message id (`worker::fetch_older`), and the app asks the phone
  as soon as such a chat loads or opens, instead of never.
- `eframe`'s `glow_options` turn vsync off: a Wayland compositor stops
  sending frame callbacks to a window on a hidden workspace, a vsync wait
  there blocks the event loop and its ping replies, and Hyprland then
  calls the app unresponsive. Repaints are event-driven, so nothing spins.
- `src/voice.rs` is the codec for voice messages: OGG/Opus in and out
  (the `ogg` crate for the container, `opus` with libopus bundled and
  built by cmake for the codec, so cmake is a build dependency), plus
  the 64-bar waveform WhatsApp draws and a mono/48 kHz resampler.
  `src/audio.rs` is the sound: `Player` plays one clip at a time through
  rodio (OGG/Opus through `voice`, MP3/M4A/WAV through rodio's decoders,
  decoded on a thread, the device opened on demand and released when the
  clip ends) and `Recorder` reads the default microphone through rodio's
  `Microphone` on a thread, keeping a loudness per 50 ms for the live bars.
  Linux needs ALSA headers to build (`libasound2-dev` on Debian,
  `alsa-lib` on Arch). `Action::PlayVoice/SeekVoice` drive the player from
  the bubble; `StartRecording/CancelRecording/SendRecording` the
  microphone from the composer (the send button is a microphone when there
  is nothing to send); `Command::SendVoice` normalizes
  (`voice::normalize`, quiet takes up to just under full scale, gain
  capped), encodes and sends push-to-talk with the waveform and the reply
  quote if one was open; `Command::MarkPlayed` sends the played receipt
  once per incoming voice message. Own bubbles lay out right-aligned,
  where egui turns `ui.horizontal` right to left: rows like the voice
  player must use an explicit `Layout::left_to_right` at their own width.
  `src/ui/picker.rs` is the emoji/GIF/sticker panel. GIF search uses the
  key from Settings, else one baked in at build time from
  `ZAPFAST_GIPHY_KEY` (`option_env!`); the repository carries none. The
  phone's recently used stickers arrive in `HistorySync.recent_stickers`
  when the device links and live in the archive's `stickers` table as raw
  `StickerMetadata`, fetched when the picker opens; favourite stickers sync
  through app state (`FavoriteSticker`), which whatsapp-rust does not
  surface, so they are not shown.
- `src/paths.rs` moves a setup left by the app's earlier name
  (`fastsapp`, then `fastwhatsapp`) over once, so the linked device survives
  the rename. Migration runs after the single-instance guard and outside demos;
  keep the guard's `fastsapp:` wire identity compatible with running old copies.
- The app outlives the window, as in Spotifast: `main` runs
  `eframe::run_native` in a loop; closing the window with "keep running"
  on sets `hide_intent`, the window is destroyed, and a headless loop keeps
  calling `App::background_frame` (the link, the archive, the tray) until
  the tray, a clicked notification, or another launch sets `wants_show`,
  when a new window is made. `src/tray.rs` is the Linux status notifier
  (ksni), `src/tray_native.rs` the Windows and macOS item (tray-icon; on
  macOS made with the first window and pumped by `tray::idle` while none
  exists). `src/single_instance.rs` holds a loopback port so a second
  launch surfaces the first. `src/notify.rs` sends desktop notifications
  for `Event::Incoming` (live messages from others, not history) when the
  reader is away from that chat. macOS has no title bar: the content runs
  to the top. `src/macos.rs` keeps native application menus alive across window
  recreation and aligns traffic lights with the chat header. Linking retains
  `ui::titlebar_strip`; other headers reserve horizontal space for the buttons.
- Group delivery uses `archive::receipts`: save the recipients when filing an
  outgoing message, record each person's receipt, then take the least advanced
  recipient. Never promote a group from one reader, apply a receipt to earlier
  messages, or infer a historical audience from current membership. History
  trusts the phone's aggregate status, not a partial `user_receipt` list.
- Private read-state writes all use the `regular_low` app-state collection.
  `backend::read_sync` permits one at a time and backs off the whole queue after
  failure; per-chat retry queues would repeatedly rebuild the same failed
  collection. Pending positions stay in the archive until acknowledged. Snapshot
  recovery and no-progress conflict detection belong to whatsapp-rust.
- The name and icon under the phone's Linked devices come from
  `DevicePropsOverride` in `start_bot` (`os` is the name shown, the
  platform type picks the icon); WhatsApp reads them at pairing only, so a
  change shows after unlinking and linking again.
- Older history comes from the phone on demand (`Command::FetchOlder` →
  `Client::fetch_message_history` → a `HistorySync` chunk with
  `sync_type == ON_DEMAND`); the archive is paged first, the phone only
  when it is exhausted.
- Platform-specific code belongs behind `cfg` blocks; a change for one
  platform must keep the other two compiling.

Three egui pitfalls this code has already hit:

- `consume_key(Modifiers::NONE, key)` also matches the key with Shift held
  (egui only insists on the modifiers you ask for), so the composer
  inspects the events itself to tell Enter from Shift+Enter.
- `with_layout(..., Align::Center)` directly inside a vertical container
  claims the whole available height; wrap it in `ui.horizontal`.
- `ui.horizontal` inside a right-aligned bubble lays out right to left;
  see `mirrored_row`. A bubble's own click target is registered before its
  contents (from last frame's rect) so links and quotes inside win clicks.
  The empty strip beside it is registered earlier still, before the row. A
  double-click on either replies; the body keeps it for selecting the word.
- `Popup::context_menu` opens on the *response's* right-click, which those
  inner widgets take for themselves; the bubble reads the right-click from
  the input over its own rect and opens `Popup::menu` itself, so the menu
  comes up anywhere on the message.

## Branches

Use trunk-based development. Work on `main` and keep releasable work there.
Commit directly to `main`, one topic per commit, with each commit compiling and
passing the relevant checks on its own. Feature branches and pull requests are
for outside contributors; the maintainer's own work, and work done with the
maintainer, does not go through them. Do not create or push a branch unless the
maintainer explicitly asks for one.

Keep `main` linear. Squash outside pull requests into one focused commit while
preserving contributor credit. Never create or push merge commits. When
updating a local checkout, use fast-forward-only pulls and rebase unpublished
local commits if needed. Before pushing, verify that the commits being added
contain no merge commits. Rewriting published history requires explicit
maintainer approval, an exact force-with-lease guard, and a recovery ref.

Every normal release, including a release candidate or other prerelease, must
tag a commit already pushed to and reachable from `origin/main`. A release
branch is allowed only for an explicitly requested backport to an older
supported line. Prefer fixing forward on `main`; do not create backport or
release branches speculatively.

## Releasing

Never use em dashes in user-facing writing, including release titles, release
notes, and agent responses. Use commas, colons, parentheses, or full stops.

Before writing release notes, read the previous two stable releases of
`../spotifast` and match their style: a short plain-language summary, `New`
and `Fixed` sections with bold user-facing results, a `Thanks` section, and
a full-changelog link. Credit who did what on the relevant item, with issue
or PR numbers, and acknowledge reporters separately from implementers.
Include screenshots or short videos of the main features, especially Omarchy
theme integration when relevant. Capture only synthetic offline demo content,
never real chats. Upload the media as assets of the GitHub release and link
those URLs from the notes; never commit screenshots or recordings to the
repository. Verify every media link and do not leave generated notes
in place. Describe known limitations honestly.

Do not cut a release for every fix. Work accumulates on `main` until
there is something substantial to announce: a feature, or a batch of
fixes worth a changelog entry. Five patch releases in a day is what this
rule exists to prevent. The exception is a regression in something just
released, which goes out as soon as it is fixed.

A release is not finished when the tag is pushed. Do these in order:

1. From a clean, up-to-date `main`, bump `version` in `Cargo.toml` and update
   `Cargo.lock` with a build. Run the full checks, commit, and push `main`.
   Before tagging, verify the release commit is reachable from `origin/main`
   so the binaries report the right version and the release contains the
   canonical history.
2. Tag `vX.Y.Z` and push the tag. Wait for every platform build, artifact,
   and `checksums.txt`.
3. Replace the generated GitHub notes with written release notes. Start with
   a short summary, group user-visible changes under headings such as `New`
   and `Fixed`, credit contributors and reporters where it helps, and end
   with a full-changelog link comparing the previous tag. Write about what
   changed for the user, not the commit history.
4. After the release files exist, update both `zapfast_version` in
   `docs/_config.yml` and the version menu in `docs/_data/versions.yml`.
   The menu lists only the current version, which points to `/download/`,
   and the Changelog link; do not add older versions to it. Never point the
   download page at files that do not exist yet. Set `release_asset_prefix` to
   `zapfast` and `release_app_name` to `ZapFast` only once those assets exist.
5. Update the AUR packages from the templates in `packaging/arch/`. The shared
   packaging workflow generates versions, hashes and `.SRCINFO` after the
   release exists, and publishes when `PUBLISH_AUR` and the required secrets
   are configured. Otherwise use `native-packages` to build, stage,
   review and publish the generated recipes; see `PACKAGING.md`. Validate
   native builds with `makepkg -f`. A recipe-only `zapfast-git` change does
   not require an application release.

## Definition of done

- Add focused tests for changed behaviour. The `demo` feature carries sample
  data and a headless layout test of every screen (`src/demo.rs`); extend
  the sample when a new kind of content or state is added, and use
  `--demo-shot` to look at the result.
- Update the README when user-visible behaviour, settings, files, or network
  access changes.
- Run the full checks before finishing:

  ```sh
  cargo fmt --all --check
  cargo clippy --locked --all-targets -- -D warnings
  cargo clippy --locked --all-targets --all-features -- -D warnings
  cargo test --locked --all-targets
  cargo test --locked --all-targets --all-features
  RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
  ```

  Do not weaken a lint, delete a test, or add an `allow` merely to make
  them pass without explaining why the rule does not apply.
- Report platform coverage honestly: say what was run and what was only
  compiled.
- Never log message contents, phone numbers, keys, or QR payloads at a
  level that ships. The log file is meant to be attached to bug reports.

## Disk use

Build caches save hours of recompiling, so keep them, but keep them small:

- Use one build cache per project: `target/` in the main checkout. Git
  worktrees and parallel agents set `CARGO_TARGET_DIR` to that directory
  instead of building their own; a fresh target costs 20 GB or more.
- Never put build output or large scratch files in `/tmp`. It is a small
  in-memory filesystem with a per-user quota, and filling it breaks every
  shell on the machine.
- Rotate the cache: `cargo sweep --time 14` (from `cargo install cargo-sweep`)
  removes artifacts unused for two weeks. If `target/` still exceeds about
  60 GB, run `cargo clean`.
- Delete one-off QA, packaging, and release-validation directories (under
  `.cache/` or `~/.cache/`) once their result is recorded.
