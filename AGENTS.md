# ZapFast agent guide

ZapFast is a small native WhatsApp client: Rust, egui, and the
[whatsapp-rust](https://github.com/oxidezap/whatsapp-rust) library for the
protocol. These notes are for coding agents and new contributors.

## Product boundaries

- Keep it a small native client. No browser engine, no telemetry, no
  hosted backend, no ZapFast-operated account system. Features never send
  message content to a third party.
- WhatsApp only. ZapFast is a client for WhatsApp accounts and will not
  speak other networks or protocols (Matrix, Signal, Telegram, IRC, XMPP)
  or bridge to them.
- No automation, ever. ZapFast is for a person reading and writing their
  own chats by hand. It will not have bots, an MCP server, or any API,
  command-line, plugin or scripting interface that reads, exports or sends
  messages for another program, nor scheduled, bulk or automatic replies.
  WhatsApp's terms forbid automated use, and it bans accounts for it, so such
  a feature would put every user's account at risk. Opening a chat from a
  link or `zapfast <number>`, which only shows the chat to the person, is
  not automation.
- Do not copy dependency source into this repository. For a dependency fix,
  use a fork under `github.com/crmne/` pinned to a commit and contribute the
  fix upstream; egui, winit and rodio already come from such
  forks. Only the maintainer creates those forks; in a contributor's pull
  request, send the fix upstream and ask for it to be carried. Use the fork
  until a release includes the fix. The maintainer has explicitly approved
  pinned commits from `github.com/oxidezap/whatsapp-rust` once our carried
  fixes have merged upstream. Never point a dependency at any other fork
  or source: `.github/scripts/check-dependency-sources.py` fails CI on one.
- Screenshots, captures, comparison pages and recordings made for a review
  go in the pull request, never the repository:
  `.github/scripts/check-review-evidence.py` fails CI on any file under
  `docs/assets/`, or image or video elsewhere under `docs/`, that no page,
  top-level file or packaging metadata uses.
- The protocol comes from whatsapp-rust. Do not reimplement pieces of it
  here, and do not advertise a capability merely because a protobuf field
  for it exists.
- Nothing ships for one platform only. A feature or theme works on Linux,
  Windows and macOS, through each platform's own mechanism where they differ
  and a sensible fallback where one lacks it.
- Fewer settings. Prefer a good default to a new setting, and never add a
  setting per preference; a change that needs a new setting needs the
  maintainer's approval first.
- Pull requests leave the translation template and catalogs
  (`assets/i18n/`) alone unless the change is a translation itself: the
  maintainer regenerates them on `main`, and regenerated catalogs bury the
  real change and conflict with every other pull request.
- Do not broaden a task into adjacent features or a general refactor.
  Preserve existing user behaviour unless the task changes it.
- Current limitations are not product exclusions. Before marking a report
  out of scope, identify the explicit boundary it conflicts with and check
  the relevant implementation and reported version. A missing feature,
  download limit, or stale guide does not establish a permanent boundary.

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
  thread; `src/backend/worker.rs` runs there. Each account owns one
  `Backend`. It owns the whatsapp-rust `Bot`, the message archive,
  downloads, and profile pictures. The two sides talk only through
  `Command` (interface to runtime) and `Event` (runtime to interface);
  every event wakes the window through `Waker`.
- Several WhatsApp accounts may be linked in one process. Each account has
  its own folder under `state/accounts/<id>/` (`session.db`, `archive.db`,
  stickers) and `cache/accounts/<id>/` (media, avatars). Never mix files,
  caches, or SQLCipher keys across accounts. `ChatId` is unique only inside
  one account. Notifications, tray clicks, and search hits always carry an
  `AccountId`. `src/app.rs` is the process shell (theme, window, tray,
  updates). Each `Account` in `src/account.rs` owns a `Backend`/`Worker`.
  Views draw the active account through `App`'s `Deref` to `Account`.
  A navigation rail (`src/ui/rail.rs`) runs down the left of the chat list
  on every platform, 52 points wide and a shade darker than the list:
  Chats and Calls at the top, the one on screen lit (Calls and Starred
  are left through Chats or Escape, never toggled); Starred, Archived and
  Locked (while there are such chats; they open in the list column),
  Settings and our own avatar at the bottom. While the list is hidden the
  chats' avatars fill it between the two (`chats::compact_list`); beside the
  list they would only repeat it. On macOS the traffic lights reach
  past it over the header beside it, which leaves `rail::lights_overhang`
  free; a narrow window hides the rail with the list. The Calls tab keeps
  the open chat beside its history (`App::chat_view`), and a picked call row
  opens its chat there. Our avatar opens the account switcher
  (`src/ui/accounts.rs`): only the accounts (picture, name or number, unread
  chats, a check on the one on screen) and Add account. A dot on the avatar
  means another account has unread chats. Events from an account that is not on screen are
  applied with `App::events_hidden` set: they update that account only, never
  the window's composer, dialogs, playback, or read state (a hidden account's
  remembered chat is not being read, so it sends no receipts). Process-wide
  settings (download folder, proxy) go to every backend. `paths.rs` moves a
  single-account layout into `accounts/1/` at startup, after logging starts:
  it refuses when anything is in the way (a linked `accounts/1/` wins, and
  files beside it stay untouched: only an older build makes them), copies and reads back the archive's
  keyring key before moving it, and moves SQLite side files before their
  database. Removing an account deletes its folders after its backend has
  stopped, then its keyring entry.
- `src/archive.rs` is the SQLite store of chats, messages, contacts, and
  privacy-id mappings. The phone sends recent history at link time and can
  supply older history on request, but recovery is not guaranteed. Preserve
  the local archive. It keeps each message's raw protobuf because
  the keys to fetch an attachment live in it. `src/archive/encryption.rs` opens
  the archive with SQLCipher and a random key stored in the OS keyring. Plaintext
  migration checkpoints the old WAL and verifies an encrypted staging file before
  atomic replacement. A locked or missing key stops linking; never fall back to
  a disposable archive. Tests use fixtures and mock credentials only.
- `src/model.rs` holds the app's own types. Views never touch a protobuf;
  the worker translates in `classify()` and `parse_conversation()`.
- A motion photo is a photo plus a short clip sent as a second message in an
  `associatedChildMessage` wrapper. The clip is never a message: its bare
  video message goes to the archive's `motion_clips`, keyed by chat, photo id,
  and sender, and only the photo's own sender gives it a clip. `Content::Image`
  carries `motion`; the badge downloads the clip on a click
  (`Command::DownloadMotion`, cache only, same limit) and plays it muted through
  `animation::frame`; the image preview plays it through `Action::ExpandVideo`. Other wrapped children are not handled.
- Individual **Delete for me** uses `Command::DeleteLocal` to send through
  `chat_actions().delete_message_for_me`, removing the local copy only after
  WhatsApp accepts it. `DeleteMessageForMeUpdate` applies deletions from other
  devices. The encrypted archive retains pending requests and deletion barriers
  so reconnects retry uncertain requests and history cannot restore deleted
  messages. Confirmed deletions with failed local cleanup are repaired without
  another send. Recovery and callbacks remain bound to the originating account.
  This sync landed after 0.19.0; that release's local-only behavior was a missing
  integration, not a product exclusion. Whole-chat deletion and clearing sync too.
- Favorite chats sync with the phone through the `favorites` app-state action
  (RegularHigh), which carries the whole ordered list: `Event::FavoritesUpdate`
  replaces ours and `send_app_state_action(&schemas::FAVORITES, ..)` writes it.
  `archive/favorites.rs` keeps the list in order with each entry's JID as the
  phone named it, plus a queue of changes made here; a phone list applies
  (unless older than the newest applied) and the queue replays on top.
  `backend/worker/favorite_chats.rs` sends one list at a time with backoff and
  never before the phone's list is known: the first connection reads
  RegularHigh once as a snapshot, and its completion on the same queue as the
  replayed mutations means a phone without favorites. A blind write would
  replace the phone's list. Channels are never favorites. The Favorites chip
  follows the list order; pins stay global and first in every chip.
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
- Calls use whatsapp-rust's `voip` feature (`Client::voip()`): signaling,
  keys, the relay transport and the codecs are the library's, never ours.
  `src/calls.rs` is one call's phase machine, fed only by the protocol's own
  stanzas and media events (a relay alone is not an answered call). The one
  local phase is `Placing`: the window draws it ("Connecting…") the moment a
  call button is pressed, under a generation it takes from
  `calls::next_generation` and hands the worker in `Command::StartCall`, with
  a cancel flag that keeps a call hung up while placing from being offered;
  every way placing stops short answers with that generation failed. Ringing
  is the peer's `<preaccept>`. A call hung up or declined here leaves the
  screen at once, any other end after `CALL_FAREWELL` (1.5 s).
  `src/call_tones.rs` synthesises the ETSI ringback (while `Ringing`) and
  the incoming ringtone (while the call's notification may sound) on the
  default output, and only tracks them in tests and demos;
  `src/call_audio.rs` bridges the engine's 16 kHz frames to a
  fastframe-audio microphone and a speaker output of the call's own behind
  stable channels, so a device change rebinds a stream instead of the call;
  its pickers list and open devices by the system's id (`DeviceId`).
  `call_audio/echo.rs` cancels the speaker's echo from the microphone through
  `call_audio/chain.rs`: sonora's high-pass filter, WebRTC's AEC3
  (`sonora-aec3`'s block processor, driven directly and framed as sonora's
  wrapper frames it, because sonora's pipeline takes no AEC3 configuration),
  then sonora's pipeline for noise suppression and AGC2 only; all Rust, no
  C++ or system library. AEC3 runs `chain::tuned`: a 60-block main filter and
  an adaptive reverberation model starting at a one-second tail
  (`ep_strength.default_len = -0.985`), measured on a recording of desk
  speakers in an untreated room (11 dB of echo removed with the stock
  settings, 26 with these); a fixed long tail instead suppressed double talk
  in a synthetic reverberant room. The call's `MixOutput` hands every block
  it plays to a `Monitor` (`echo::Tap`) that mixes it mono into an `rtrb`
  ring on the audio thread, never waiting or allocating, and the microphone's
  reading thread drains it and runs both sides in 10 ms chunks at 16 kHz
  before packing the engine's frames. The reference is taken as played, not
  as the engine queued it, so the queue's depth never shifts the alignment;
  the devices' reported latencies are only AEC3's starting hint, and it finds
  the delay itself (within about 600 ms), so the microphone's reader never
  waits for the engine: `call_audio::offer` lets the oldest queued frame go
  instead, or a call answered before its media is up leaves the microphone a
  second behind and the canceller cancels nothing. One `FarEnd` per call
  links the two sides across device changes.
  `ZAPFAST_CALL_AUDIO_DUMP=<folder>` (`call_audio/dump.rs`, never by
  default) writes each microphone stream's reference, raw microphone and
  output as 16 kHz float WAV in the canceller's 10 ms chunks, plus the order
  they were fed in; `examples/aec_replay.rs` replays one exactly (a test
  checks it is sample for sample) with other AEC3 settings (`--set`, by
  `chain::apply_setting`'s names), scores far-end-only seconds and a voice
  mixed in, and measures the true echo delay by cross-correlation. Recordings
  are real voices and never enter the repository:
  `ZAPFAST_ECHO_RECORDING=<…-reference.wav>` runs the recorded-room test on
  one, which passes untouched without it. AGC2 applies digital gain only, from 0 up to 20 dB (normalize's
  cap), never the system's microphone volume. Headphones need no setting:
  AEC3's transparent mode stops suppressing when it hears no echo. `backend/worker/calls.rs` owns at most one
  call per account; the window (`app/calls.rs`) holds one call across all
  accounts with its `AccountId`, routes call actions to that account's
  backend, and declines another account's offer while one is up.
  Group calls are experimental: `calls::GROUP_CALLS_ENABLED` is `true` since
  0.20. A three-person test carried audio every way and video both ways, but
  our video froze every few seconds, since the library sends video FEC in
  1:1 calls only (a guessed hop-by-hop group FEC, whatsapp-rust branch
  `voip-group-video-fec`, made the SFU drop our media altogether). The switch
  stays, kept compiling and tested both ways
  (`calls::override_group_calls` sets it per test thread). Read it through
  `calls::group_calls_enabled()`. While it is off a group has no call
  button and no call back (`ui::conversation::callable`, `call_menu`, the
  Calls page row), a clicked call link toasts `calls::CALL_LINKS_OFF`, and
  the worker refuses a group `StartCall` or `JoinCallLink`. An incoming
  group offer never becomes `Worker::call`: `ring_on_phone_only` keeps it in
  `Worker::phone_only_calls`, sends WhatsApp nothing (not even while busy),
  so the phone keeps ringing, and emits `Event::PhoneOnlyCall`, which
  `App::handle_phone_only_call` shows as a notification and toast saying
  who is calling which group and to answer on the phone. Its end
  (signaling, `MissedCall`, `CallEndedElsewhere`) files it in the group
  through `file_finished_call`, as a call that rang here would be, and
  takes the notification back. Turning the constant on restores all of it.
  Group calls: an offer carrying a roster (`IncomingCall::group`) or a `group-jid`
  rings under its group (a call among several people with no group, under its
  creator's chat), named by `chat_title_of` with the member who rang, and is
  answered through the same `voip().accept`, which joins the shared relay and
  epoch key; the library mixes everyone's audio into the one speaker stream.
  `calls::GroupRoster` keeps the newest roster snapshot by transaction
  (`CallAction::GroupUpdate` while ringing, `CallEvent::GroupUpdated` once up),
  and one member's `<reject>` never ends the call. The worker files roster ids
  under canonical chat ids and drops our own entry (`canonical_call`), and the
  call's entry goes in the group under the member who rang. The library
  reports no per-person speaking level or mute, so the tiles show neither.
  Group video frames name their sender (`VideoFrame::sender`):
  `decode_remote` keeps a decoder per sender (`MAX_REMOTE_STREAMS`, the oldest
  let go), converts at tile size, and `VideoTick::RemoteFrom` reaches the
  window as `Event::CallVideo { from }` under the member's chat id;
  `Participant::video` follows their first frame, their own video state and
  the roster. We have a tile of our own in the grid (`App::call_self`): our
  camera, else our photo, with a muted sign when we are muted, and
  `request_peer_keyframe` does nothing in a group call. A call link
  (`safety::call_link`, `/voice/`, `/audio/` or `/video/`) clicked in a message
  is joined with `voip().call_link(token, ..)` on a task of its own, since the
  waiting room may hold it indefinitely; its answer returns as
  `CallRuntimeEvent::LinkJoined`, and hanging up aborts it. Such a call has an
  empty chat and `GroupCall::link`, and its local entry is filed under
  `model::CALL_LINK_CHAT`, an id no chat has: it reaches the Calls page
  (`Event::CallLog`) and nothing else, and its row there opens nothing. The library has no
  way to join a group's ongoing call uninvited, so there is no Join banner.
  A group's header has the call buttons a person's has: `Command::StartCall`
  on a group chat goes to `Worker::start_group_call`, which starts the call
  bound to the group (`voip().group_call_by_id(..)`, with video when asked) on
  a task of its own, since the library returns only after the call service's
  ack and the relay; its answer returns as `CallRuntimeEvent::GroupStarted`.
  The window draws it as the group's call from `Placing`; it reads Calling
  (`Dialing`) until someone but us joins the roster (`GroupRoster::own`), then
  goes active once the media is up. WhatsApp gates group calls from linked
  devices per account (`enable_web_group_calling`, logged once per connection
  and at each start) and refuses an offer with 427 ("group call not enabled")
  where it is off. The button is offered either way. whatsapp-rust does not
  report that code: the 427 ack's `group_info` lacks the counters it requires,
  so it fails as a call-service response error, which `calls::GroupRefusal`
  reads as the 427 when the gate is off or the library names the code. Then
  the window says `calls::GROUP_CALLS_NOT_ENABLED` (someone else in the group
  can call it, and the call is answered here); other failures stay generic.
  A group's voice call cannot gain video. Call
  updates are withheld during privacy recovery like other private content,
  and a locked chat's call shows only "Locked chat" until its folder opens.
  Calls open the system default devices; the call screen's pickers apply to
  the running call only and are not saved. Video: `src/camera.rs` reads
  cameras through linuxvideo on Linux (pure Rust, no build-time bindings) and
  nokhwa on macOS and Windows, converts every layout to YUV 4:2:0, and the
  call's capture thread encodes it with openh264 (Constrained Baseline, 720p
  at most, 25 fps) into the library's timed `VideoSource`. `src/video_rate.rs`
  adapts the bitrate (from 700 kbit/s) and the step of size and frame rate
  (480p, 540p and 720p, all at 25 fps, preferring motion to size) to the peer's RTCP loss, its REMB, dropped video,
  queue delay (the round trip from its reports' LSR/DLSR above the call's
  shortest, since a deep buffer holds seconds before it loses), and the rate
  the peer receives (word 5 of WhatsApp's 24-byte extension to its video
  report block; below 75% of what the encoder made over the last few seconds,
  never the rate, which a calm picture stays well under, it caps us at 90% of it, once word 0, the
  bytes received, passes 500 kB, since it starts from nothing), and climbs
  quickly back only to just under where loss was last reported, slowly past it;
  loss from 2% to 10%, and each keyframe request, hold it from rising for
  `CONGESTION_HOLD` without cutting it (an iPhone asks once near the start of
  a clean call too);
  keyframes go out every five seconds unasked, and on the peer's request. In a
  1:1 call the library sends XOR repair packets (FEC) beside the video, which an
  iPhone relies on since it never NACKs; the rate chosen is the total on the
  wire, so each heartbeat measures FEC's bytes against the video sent
  (`RateControl::on_fec_sent`, `MediaStats::video_fec_bytes_sent`) and the
  encoder's target leaves room for them. The
  call's heartbeat logs both directions' frame counts, keyframes and decode
  failures, and the peer's H.264 profile (`src/call_diag.rs`);
  `ZAPFAST_CALL_VIDEO_DUMP=<folder>` writes both directions' access units as
  raw H.264 for offline analysis, never by default. The camera is thinned to every nth frame of its measured rate and
  stamped from capture time, never thinned by elapsed time; the peer's access
  units decode on a thread of their own and reach the window as
  `Event::CallVideo` frames. A camera that cannot open leaves a video call
  receive-only rather than refusing it. Call history is `Content::Call`
  messages: the phone's own `callLogMessage`s and `CALL_MISSED_*` stubs, plus
  local entries (ids prefixed `CALL_ENTRY_PREFIX`) for calls made here,
  `MissedCall`, `CallLogSync` and `CallLogHistory`. A local entry is skipped
  when the chat already has a call the same way within `CALL_ENTRY_WINDOW`,
  and the phone's own message, filed later, retires it
  (`retire_local_call_twins`); local entries are never sent, read-receipted, reacted to or forwarded. The
  Calls page reads `archive::call_log` per account.
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
- Updates come from fastframe-update: `src/updates.rs` holds ZapFast's
  `UpdateConfig` (legacy `fastsapp` names, the publisher key) and the
  proxy-aware client. It downloads verified GitHub releases and hands
  installation to a helper after an explicit restart action. `main` calls
  `fastframe_update::intercept` before anything else, so `--apply-update`,
  `--update-receipt` and `--update-error` keep working with older releases'
  helpers (`tests/update_flags.rs`). Portable releases carry
  `packaging/zapfast-portable.txt`; the Windows installer has its own marker.
- Custom themes come from fastframe-theme (`theme::Catalog`, ZapFast's
  `fastframe_theme::Palette` impl in `src/theme.rs`): it scans local JSON
  palettes off the UI thread, and the app caches the last usable choice in
  settings, with the palettes shared with Spotifast embedded as defaults. On Linux filesystem notifications reload the catalog and the active
  Omarchy palette without a repaint timer; following Omarchy does not require
  packaged assets. Changing `contrib/omarchy/zapfast.json.tpl` means saving the
  old text in `contrib/omarchy/previous/` and listing it in
  `omarchy_previous_templates`, so untouched installed copies are upgraded. Native packages ship optional hooks and templates, preserving
  existing per-user files. `reload-themes` uses the single-instance channel
  without opening a window.
- `src/theme.rs` owns colours, fonts, and icons; `src/ui/widgets.rs` the
  shared controls. New icons go in `assets/icons/` as 24px Lucide-style SVGs
  and in the `fastframe_icons::icons!` table; an icon fastframe-icons already
  ships is named there as `lucide "name"` instead of copied.
- `src/markup.rs` turns WhatsApp's text markup, links, and mentions into an
  egui `LayoutJob`; `src/emoji.rs` hands emoji to fastframe-emoji, which
  swaps every emoji for a placeholder glyph at layout time and paints the
  platform's picture over it afterwards (Apple Color Emoji, Segoe UI Emoji
  through DirectWrite, or the desktop's bitmap emoji font, with the bundled
  Noto behind them). New pictures are drawn on its worker thread; tests and
  demo builds draw them inside the frame. The interface font is the
  platform's own (`fastframe_fonts::Primary::System`), and Inter in tests.
  fastframe-emoji's `EmojiPlugin` (added in `App::attach`) colours the
  emoji in every other egui text: labels, menus, tooltips, text fields. It
  leaves placeholders and `editor_job` glyphs alone, so the two never paint
  one emoji twice. Message text and chat names still go through
  `widgets::line` / `widgets::rich_text` or `markup::layout`: their
  placeholders keep the emoji-only sizing and let `transcript::refine`
  put copied emoji back.
- `src/animation.rs` plays animated stickers and GIFs: WebP/GIF frames
  decode in-process, and so do MP4s (the `mp4` crate demuxes, `openh264`
  decodes the H.264 WhatsApp uses, samples converted from AVCC to Annex
  B); `ffmpeg` is only a fallback for other codecs. `openh264` compiles
  its C++ from source with the C++ compiler of the host; `nasm` is
  optional and only adds the SIMD paths (the AUR recipes leave it out,
  the build works without it). Frames become textures on the interface
  thread and are dropped when unseen. A paused animation decodes only its
  first frame, the poster, and the rest once it plays: full decodes of a
  picker's paused stickers overran the frame budget and evicted each other.
- `src/video.rs` plays other videos inside their message, one at a time,
  with the same `mp4` and `openh264` pieces: a thread decodes from the
  keyframe before the start (openh264 must not flush after each packet or
  B-frames stop it) and streams scaled frames with presentation times; the
  interface thread shows the due frame in one texture. `mp4` reads the AAC
  track too, symphonia's AAC decoder decodes it, and a rodio player on the
  shared output plays it; its
  position steers the clock. rodio's own MP4 reader (symphonia 0.5's isomp4)
  is not used for it: it refuses WhatsApp's videos, whose `esds` carries a
  custom SLConfigDescriptor (#265). Unsupported formats
  go to the system player. Playback reads a downloaded local file; the 64 MiB
  attachment limit applies only to automatic downloads (`manual: false` on
  `Action::Download`/`Command::Download`), never to a click or a play, and
  not to the video decoder. Every download's deadline grows with its size
  (`attachment_deadline`). `Action::PlayVideo/SeekVideo/ToggleVideoSound` drive
  it; leaving the chat stops it and an unseen video pauses. Round video
  messages (PTV) are `Content::Video { note: true }` and draw as circles.
  `Action::CycleVideoSpeed` (the chip, or S full-window) runs the `Clock` at
  1x, 1.5x or 2x; above 1x the sound is queued through `timestretch::Stream`,
  the voice messages' pitch-keeping stretch run as the sound plays. It holds
  only the frames its next search can reach, never the rest of the track,
  searches on the channels' average and copies every channel, so stereo stays
  stereo. A second heard covers `factor` seconds of video (`sound_position`),
  so the sound keeps steering the clock at any speed.
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
  Messages themselves are swept by a drag that starts on the strip beside
  the bubbles (which senses drags beneath the text), or anywhere on a row
  while selecting (the row's pick target sits above the text). `App::sweep`
  keeps the anchor and the selection it started from; the view maps the
  pointer's y to the last laid-out row above it each frame and the app
  selects by message order, so rows the list skipped during edge scroll
  count. Releasing the button ends the sweep, whichever widget held it.
- Group names and members come from `groups().get_metadata`, asked one
  turn at a time (two per 5 s tick, `pump_group_info`): dozens of unnamed
  groups arrive with history sync and a burst of queries hits the
  server's rate limit, which once left groups called "Group" forever.
  Failures back off (30 s doubling, seven tries); item-not-found,
  forbidden and not-authorized are final and stop the asking.
  The same metadata stores `is_locked` and our admin role
  (`chats.info_locked`, NULL until known, and `chats.group_admin`);
  `Chat::can_edit_info` gates renaming and the group photo in the group
  dialog. A rename lands only after WhatsApp accepts it, and bumps
  `subject_generation` so a metadata answer asked for earlier cannot restore
  the old subject. A refusal re-asks the metadata to relearn the rights.
- New chat's **New group** turns the same dialog into steps
  (`new_group::Draft` on the `Account`: choosing with check boxes and chips,
  then naming with an optional photo picked and cropped as the group dialog's
  is). `Command::CreateGroup` calls `groups().create_group` with us as the
  creator; the library reads a refused `<participant error="403">` in its
  answer as a member, so the task fetches the metadata once more and the
  people picked but missing are reported by name. `group_created` files the
  chat named, with its members and our admin rights, before
  `Event::GroupCreated` opens it.
- Group management lives in the info panel (`src/group_admin.rs`, the worker's
  `backend/worker/group_admin.rs`). The metadata's permissions and admins are
  filed as `chats.group_settings` (JSON, NULL until known; `Chat::group_settings`),
  read by `settings_of`: only an explicit `all_member_add`/`all_member_link`
  lets members add people or see the link, WhatsApp's default being admins
  only. `Chat::can_add_members`, `can_share_invite_link` and `can_manage_group`
  gate the panel's Add members, Invite via link, Group permissions (an
  `InfoView::Permissions` page: `set_locked`, `set_announce`,
  `set_member_add_mode`, `set_membership_approval`) and each member's menu
  (`promote_participants`, `demote_participants`, `remove_participants`);
  the worker checks the same rights again. Add members is New group's
  `Draft` made with `Draft::adding` (the choosing step alone, without the
  group's members), sent with `add_participants`, whose per-person error
  codes name who was refused and why (403 privacy, 408 recently left; 409,
  already in, counts as added). Each answer returns as
  `Command::GroupManaged`; `group_managed` files only what WhatsApp accepted
  and bumps `settings_generation`, so metadata asked for earlier (the library
  joins a query in flight) cannot undo it, and a refusal re-asks the
  metadata. The invite link (`groups().get_invite_link`, `reset` revokes it)
  is a secret: `group_admin::InviteLink` hides it from `Debug`, it is never
  logged or stored, and the window keeps it only while the panel shows its
  group (`App::info_invite`). The library has no simple way to send an
  invite message, so people WhatsApp refuses are named and pointed to the link.
- Mentioning everyone uses the visible body token `@all` and bit 1 of
  `ContextInfo.non_jid_mentions` (`src/mentions.rs`). Localized words
  (`@todos` in Portuguese, and the one-word alias of each other language)
  are composer triggers only; `encode_composer_mentions` rewrites them to
  `@all` when `Chat::can_mention_everyone` allows it: the sender can post,
  the member list is known, and the group has at most 32 people or the
  sender is an admin. The stored mention id is `mention-all`, not a JID.
  Forwards and quoted copies clear that bit so the destination is not
  notified again. A plain `@all` without the bit stays ordinary text.
  Live edit events replace content and mentions together; caption edits keep
  downloaded media paths. Derived archive backfills skip edited rows because
  their retained raw protobuf still contains the original body and mentions.
- A download that answers 403/404/410 goes through
  `client.media_reupload().request(..)` (a server-error receipt; WhatsApp
  has the phone re-upload and answers with a fresh `direct_path`) and is
  fetched once more before the bubble reports "No longer on WhatsApp's
  servers". A chat sticker only the picker asked for skips the re-upload
  (#405): the picker fetches through `sticker_pace`, two at a time, and a
  failed one is not asked again that session. Download failures never toast; they live in the bubble as
  "... · click to retry". Copied text is refined by
  `transcript::refine`: emoji placeholders map back through each row's
  `placements`.
- History sync can bring a chat with a name and no messages at all; a
  history request for such a chat is anchored at the present with an
  empty message id (`worker::fetch_older`), and the app asks the phone
  as soon as such a chat loads or opens, instead of never.
- A Wayland compositor may stop sending frame callbacks to a window it isn't
  showing without saying so (Hyprland does for a window covered by a
  fullscreen or maximized one, #190), and a vsync swap there blocks the event
  loop and its ping replies, so the compositor calls the app unresponsive.
  The shared egui fork (crmne/egui apps-0.36.2, emilk/egui#8631) presents
  without a blocking swap on Wayland, paces frames by frame callbacks, and
  runs only `App::logic` when a redraw is 250 ms overdue; the winit fork
  (apps-0.30, rust-windowing/winit#4709) reports `suspended` as `Occluded`.
  Vsync stays on elsewhere. Spotifast, RekordFlash and TonePush pin the same
  two revisions; move them together. Repaints are event-driven, so nothing
  spins.
- `src/voice.rs` is the codec for voice messages: OGG/Opus in and out
  (the `ogg` crate for the container, `opus` with libopus bundled and
  built by cmake for the codec, so cmake is a build dependency), plus
  the 64-bar waveform WhatsApp draws and a mono/48 kHz resampler.
  `src/audio.rs` is the sound. Every sound and recording goes through
  fastframe-audio, never rodio's or cpal's own devices: rodio only decodes
  and mixes. `audio/output.rs` keeps one shared output on a thread of its
  own (`open_output` hands out its mixer, to connect rodio players to); it
  pauses while nothing is connected and lets the device go after a long
  pause, so a player is dropped when its sound ends. Its `Mix` renderer
  stands in for fastframe-audio's `MixerRender`, which mixes with rodio 0.21,
  until that follows rodio's newer line. `audio/microphone.rs` is an input
  read without blocking. `main` names the app for volume mixers
  (`fastframe_audio::set_app_info`) before any device opens; a call's own
  speaker and microphone are the "Call" stream with the communication role,
  so the system can lower other sound during a call. `Player` plays one clip at a time (OGG/Opus through
  `voice`, MP3/M4A/WAV through rodio's decoders, decoded on a thread) and
  `Recorder` reads the default microphone on a thread, keeping a loudness
  per 50 ms for the live bars. Linux needs ALSA's and libpipewire's headers
  and clang to build (`libasound2-dev libpipewire-0.3-dev clang
  libclang-dev` on Debian, `alsa-lib pipewire clang` on Arch). `Action::PlayVoice/SeekVoice` drive the player from
  the bubble, and a clip that ends hands its message back
  (`Player::take_finished`) so the app plays the next unheard voice message
  of the same run (`App::next_voice_after`), keeping other apps' media paused
  in between; `StartRecording/CancelRecording/SendRecording` the
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
  `StickerMetadata`, fetched when the picker opens. Favourite stickers sync
  both ways through `FavoriteStickerUpdate` and `schemas::FAVORITE_STICKER`;
  `backend/worker/stickers.rs` and `archive/stickers.rs` retain pending local
  changes and recover favourites synced before support was added.
- `src/paths.rs` moves a setup left by the app's earlier name
  (`fastsapp`, then `fastwhatsapp`) over once, so the linked device survives
  the rename. Migration runs after the single-instance guard and outside demos;
  keep the guard's `fastsapp:` wire identity compatible with running old copies.
- The app outlives the window, as in Spotifast: `main` hands the app to
  `fastframe_shell::Shell`, which runs `eframe::run_native` in a loop through
  App's `Resident` impl; closing the window with "keep running"
  on sets `hide_intent`, the window is destroyed, and a headless loop keeps
  calling `App::background_frame` (the link, the archive, the tray) until
  the tray, a clicked notification, or another launch sets `wants_show`,
  when a new window is made. `Shell::run` marks its thread as the UI thread,
  where `fastframe_file::write` panics in debug: settings and the roster go
  through `App::files` (a `fastframe_file::Writer`) with `save_to`, flushed
  in `App::shutdown` and before a suspend is acknowledged, and `StallWatch`
  in `main` logs any update of 250 ms or more. The tray item is
  fastframe-tray (ksni on Linux, tray-icon on Windows and macOS; on macOS
  made with the first window and pumped by `fastframe_tray::idle` while none
  exists), and `src/macos.rs` hands its menu events to `fastframe_tray::claim_menu_event` first.
  The process Dock icon and unread badge use fastframe-dock; a Cargo run
  gets the embedded icon, while a packaged app keeps its bundle icon. The
  unread-chat count stays in ZapFast and survives window replacement.
  Clicking the macOS menu-bar icon opens its menu; **Show or hide ZapFast**
  toggles the window, and clicking the Dock icon shows it.
  Closing keeps ZapFast running only while `Tray::is_shown`: on Linux the
  item exists before a panel shows it (a start at login beats the panel, and
  GNOME's tray watcher) and registers once one appears. A hidden start with
  an item no panel shows yet waits `TRAY_WAIT` (30 s) for one in
  `App::check_tray_wait`, then shows the window (#429). A
  hidden start makes the macOS item with `Tray::create_item`, which does not
  bring ZapFast forward; a window's `attach` makes it otherwise.
  `src/single_instance.rs` claims fastframe-instance's slot in the runtime
  directory (`Slot::at(runtime, "fastsapp")`, so requests and replies stay
  `fastsapp:show` and `fastsapp:ok` for older copies), and a second launch
  asks the first to surface over a private socket (a token-checked loopback
  port on Windows); the handler queues `ControlCommand`s and declines unknown
  verbs. The fixed port 47119 that 0.15-era copies look for stays in ZapFast,
  answered once the slot is claimed. `src/notify.rs` sends desktop notifications
  for `Event::Incoming` (live messages from others, not history) when the
  reader is away from that chat; a click carries the chat and the message
  id, so the reader lands on the announced message. macOS has no title bar:
  the content runs to the top. `src/macos.rs` keeps native application menus alive across window
  recreation; `fastframe_titlebar::align_traffic_lights` centres the traffic
  lights on the chat header. Linking retains
  `ui::titlebar_strip`; other headers reserve horizontal space for the buttons.
  On Windows and Linux, Settings > Appearance > Custom title bar
  (`settings.custom_titlebar`, off by default for Intel drivers that
  mis-draw a frameless window, Spotifast #348/#554) draws fastframe-titlebar's
  caption buttons instead: `main` calls `init` and `set_enabled` before the
  first window, `native_options` sets `with_decorations(!enabled())`, and
  `SetCustomTitlebar` replaces the window through `Closed::Reopen`.
  `ui::window_chrome` draws the buttons last each frame and keeps their areas
  on top, then `resize`. `theme::chrome_in_headers` is true for macOS and the
  drawn bar alike (banners go under the chat header); `theme::caption_footprint`
  is the drawn buttons' room (`traffic_light_inset` includes its left end,
  so the rail starts under left-hand GNOME buttons; `rail::lights_overhang`
  stays macOS-only, and headers beside the rail measure their own room),
  and `ui::caption_room(ctx, rect, min_content)` gives any header reaching
  a top corner its room beside the buttons or under them
  (`Footprint::reserve`). Every pane's top drags the window
  (`ui::caption_drag` where macOS has no drag). Mica is unaffected: the crate
  leaves the frame alone. Layout tests preview the room per context with
  `theme::preview_titlebar`, never `set_enabled`, which is process-wide;
  `--demo-titlebar` makes the choice for real in the demo.
- Group delivery uses `archive::receipts`: save the recipients when filing an
  outgoing message, record each person's receipt, then take the least advanced
  recipient. Never promote a group from one reader, apply a receipt to earlier
  messages, or infer a historical audience from current membership. History
  trusts the phone's aggregate status, not a partial `user_receipt` list.
- The app lock (`src/app_lock.rs`, `ui/lock.rs`) is a local screen lock, not
  encryption, and independent of the locked-chats code. Settings keep only a
  salted PBKDF2 verifier, checked and made on a thread. While locked,
  `ui::show` draws only the lock screen, `App::apply` drops every action
  outside `allowed_while_locked` (a clicked notification's message waits for
  the unlock), `window_focused` stays false so nothing is marked read, and
  notifications say only "New message". Unlinking (`LoggedOut`) forgets the
  password, which is how a forgotten one is recovered.
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
  when it is exhausted. Short and empty chats ask on their own, and a phone
  with nothing to add often leaves that unanswered, so only an `explicit`
  request (the reader scrolled to the top) reports a silent phone, once per
  chat until it answers or the link reconnects. A chunk saying nothing more
  remains on the phone sets `chats.history_start`, and that chat is not
  asked again.
  `ReloadHistory` requests before a selected archived message to repair gaps
  inside existing history, even after reaching the chat's start. History keeps
  edited snapshots under the protocol target id, even when the outer envelope
  names a different edit id. It uses `HistorySyncMsg.msg_order_id` to break
  timestamp ties in the archive, paging, and interface. A replay without order
  metadata must not erase it, and an unedited replay must not undo an edit.
  History `MESSAGE_EDIT` entries carry full snapshots, unlike live edit events:
  keep their envelope timestamp/order and normalize the body and target id.
  Do not require a separate original or create a bubble under the edit id.
  An edit made before linking can arrive as a `secretEncryptedMessage`
  MESSAGE_EDIT: `parse_conversation` files it as a `SealedEdit` under the
  original's id, and the worker opens it with the original's secret (the
  chunk's, the archive's, or the entry's own) before filing the snapshot;
  one it cannot open is not filed.
  Count-only diagnostics can distinguish malformed snapshots from storage failures.
- Scrolling comes from fastframe-scroll: `App::scrolling.apply` runs first
  in each unlocked frame and sets the wheel step (120 points a notch), and on
  Linux scales touchpad gestures, glides after the lift, and holds a gesture
  to its axis (Shift turns it sideways). `App::route_scroll` then keeps a
  gesture, glide included, with the pane it began over (`ScrollRoute`, #274),
  asking `Scrolling::gliding`; the image preview pans with a touchpad and
  zooms with a wheel by `Scrolling::from_trackpad`.
- Platform-specific code belongs behind `cfg` blocks; a change for one
  platform must keep the other two compiling.

egui pitfalls this code has already hit:

- `consume_key(Modifiers::NONE, key)` also matches the key with Shift held
  (egui only insists on the modifiers you ask for), so the composer
  inspects the events itself to tell Enter from Shift+Enter.
- `consume_key` matches the logical key, and with Shift held US-style
  layouts report `[` and `]` as `{` and `}` (and `=` as `+`), so a Shift
  shortcut binds both spellings: see the bracket and `Plus`/`Equals` rows in
  `ui/keys.rs`. Layouts that put another character on the shifted key never
  produce either spelling; `Alt+↑/↓` is the layout-independent way to switch
  chats. A long label in `SHORTCUTS` widens the dialog's key column and
  leaves its descriptions less room to wrap in; the dialog takes two columns
  in a wide window and scrolls in a short one.
- `with_layout(..., Align::Center)` directly inside a vertical container
  claims the whole available height; wrap it in `ui.horizontal`.
- `ui.horizontal` inside a right-aligned bubble lays out right to left;
  see `mirrored_row`. A bubble's own click target is registered before its
  contents (from last frame's rect) so links and quotes inside win clicks.
  The empty strip beside it is registered earlier still, before the row. A
  double-click on either replies; the body keeps it for selecting the word.
- `Popup::context_menu` opens on the *response's* right-click, which those
  inner widgets take for themselves; the bubble reads the right-click from
  the input over its row (the bubble and the strip beside it) inside the
  transcript viewport (the chat header shares its layer) and opens
  `Popup::menu` itself, so the menu comes up anywhere on the message.

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

1. From a clean, up-to-date `main`, bump `version` in `Cargo.toml`, add the
   release to the `<releases>` list in the Flatpak metainfo, and update
   `Cargo.lock` with a build. Write the release notes, in the style above, to
   `packaging/release-notes/vX.Y.Z.md`: the release workflow publishes that
   file as the release description, and a stable tag without it fails. Link
   screenshots at the release's asset URLs
   (`https://github.com/crmne/zapfast/releases/download/vX.Y.Z/NAME.png`).
   Run the full checks, commit, and push `main`. Before tagging, verify the
   release commit is reachable from `origin/main` so the binaries report the
   right version and the release contains the canonical history.
2. Tag `vX.Y.Z` and push the tag. Wait for every platform build, artifact,
   and `checksums.txt`.
3. Upload the screenshots to the release as assets with the names the notes
   link, then open the published release and check the text, every image,
   and every download link. Never leave generated placeholder notes.
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
- Update the docs site (`docs/`) when user-visible behaviour, settings,
  files, or network access changes. The README stays a short pointer to it.
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

Builds go through [mbx](https://mr-boxington.jdx.dev), enabled for mise users
by `mise.toml` (run `mise trust` once in each new checkout or worktree, or
mise refuses to run `cargo` there). It keeps compiled work in one shared
store, places each checkout's `target/` under a disk budget, and collects old
outputs on its own. Plain `cargo` still works for contributors who do not use
mise or mbx.

- Give each worktree and each parallel agent its own target directory. A
  worktree's own `target/` is enough, and mbx manages it; a second build in
  the same checkout uses `CARGO_TARGET_DIR=target/<name>`, which stays inside
  the managed target. Never point builds at a shared target directory: Cargo's
  lock serializes them, one worktree's test run can execute another's binary,
  and the store already shares compiled outputs.
- Never vary `codegen-units` or other compiler flags per agent. Each variant
  is a separate cache entry and fills the disk.
- Do not `cargo clean` to save space. `mbx gc --dry-run` previews collection
  and `mbx gc` runs it now; `mbx cache stats` shows what is held.
- When a build is colder than expected, `mbx explain --last` says what missed
  the cache and why.
- Never put build output or large scratch files in `/tmp`. It is a small
  in-memory filesystem with a per-user quota, and filling it breaks every
  shell on the machine.
- Delete one-off QA, packaging, and release-validation directories (under
  `.cache/` or `~/.cache/`) once their result is recorded.

<!-- github-automation: pull-requests -->
## Pull requests

This section is maintained account-wide by
[crmne/github-automation](https://github.com/crmne/github-automation) and is
replaced when that policy changes. Do not edit it here. If it does not fit this
repository, say so in a review or issue, and put repository-specific rules in a
separate section, which takes precedence.

Review evidence stays out of the repository. Before-and-after screenshots,
captures, comparison pages, recordings, and anything else made to show
reviewers a change go in the pull request description or its comments: attach
the files there, or link a page published elsewhere. Only media the project
itself uses, such as documentation figures, release-note images, and app
assets, belongs in the repository.

Dependencies come from their package registry, such as crates.io, RubyGems, or
npm, or from repositories the maintainer controls, under `github.com/crmne/`
or this repository's own organization. A git dependency, or a patch, override,
or replace entry, may point only at such a repository, and adding or changing
one needs the maintainer's explicit approval. Never point a dependency at a
contributor's fork or any other source the maintainer does not control. When a
dependency needs a fix, send it upstream and ask the maintainer to carry it in
a fork they control until a release includes it.

Reviewers treat either problem in a pull request as a blocker.
<!-- /github-automation: pull-requests -->
