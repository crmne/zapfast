# Performance baseline

This document defines a repeatable, privacy-preserving baseline for WhatsNano (the ZapFast-derived native client).

## Why this exists

Do not remove dependencies or change runtime configuration based on guesses. First measure the unmodified app, then change one thing at a time and repeat the same test on the same machine.

The long-term aspiration is **under 20 MiB idle private memory**, but this is an experimental target, not a current capability or a promise. Memory, idle CPU and battery impact are separate measurements.

## What the scripts measure

- **Linux:** root process and current descendants; proportional set size (PSS), private clean + dirty memory, and interval CPU time.
- **Windows:** root process and current descendants; private bytes, working set, and interval CPU time.
- CPU is reported as a percentage of one logical core and normalized across the machine's logical processors. The one-core value can exceed 100%.

The scripts inspect process metadata and memory counters only. They do not read chat databases, message bodies, contacts, session keys, QR codes or application logs.

Children that start or exit during a sample can make process-tree totals slightly imprecise. For an idle baseline, close transient dialogs, media previews and calls before sampling.

## Build the same release configuration

Use the Rust version pinned in rust-toolchain.toml and the same commit on both platforms.

    cargo build --locked --release

Do not compare a debug build with a release build. Do not enable the demo feature for the connected-account baseline.

## Linux

1. Launch the release binary and allow initial synchronization to finish.
2. Leave the app connected and idle for five minutes. Keep the same window size and same chat selected across runs.
3. Find the process ID and run:

    pgrep -a -x zapfast
    chmod +x scripts/perf/measure-linux.sh
    ./scripts/perf/measure-linux.sh PID 5 10

The script reports PSS and private memory in MiB. PSS is useful for comparing resident memory while avoiding counting all shared library pages as private to the application.

## Windows

1. Launch the release build and allow initial synchronization to finish.
2. Leave the app connected and idle for five minutes, using the same window size and same chat selected across runs.
3. In PowerShell, locate the process and run:

    Get-Process zapfast
    .\scripts\perf\measure-windows.ps1 -ProcessId PID -Samples 5 -IntervalSeconds 10

Private bytes are the primary Windows memory metric here. Working set is shown as additional context; it includes shared resident pages.

## Keep conditions consistent

For each before/after pair, keep these unchanged:

- Machine, OS version, power plan and power source.
- Rust toolchain, release profile and build target.
- Linked account and synchronization state.
- Window size, selected chat and UI scale.
- Number of linked accounts.
- Whether the app is visible or hidden in the tray.

Do not include calls, media playback, active downloads or initial history sync in an idle sample. Run at least three independent sessions if possible, restarting the app between sessions.

## Battery / background activity

CPU percentage is not a battery measurement. For battery comparisons, use the same machine on battery power, same brightness and power mode, and record the OS battery discharge rate over at least 30 minutes per run. Compare a no-app baseline against ZapFast idle, then compare each optimization against the same baseline. Close unrelated high-activity applications. Report whether discharge rate is measured by the OS or estimated by firmware.

## Results template

| Commit | Platform | Idle PSS / private | Working set | CPU (1 core) | Conditions |
|---|---|---:|---:|---:|---|
| Baseline | Linux | | n/a | | |
| Baseline | Windows | n/a | | | |

Record the exact commit SHA with every result. Do not claim an improvement unless the before/after runs use matching conditions and the difference exceeds normal run-to-run variation.

## First optimization experiments

Only after capturing the baseline:

1. Identify the largest retained allocations with a profiler.
2. Separate persistent idle allocations from media and call allocations.
3. Change one subsystem at a time.
4. Run the project's formatting, Clippy and test commands from CONTRIBUTING.md.
5. Repeat the same idle benchmark before considering the change successful.

## Native GUI profiler (opt-in)

The `nano-gui-profile` feature measures the native eframe callbacks without changing the GUI framework, renderer, backend runtime, or application architecture. It reports one line every ten seconds:

- GUI logic callback count and rate (calls/second).
- Average and maximum logic callback duration.
- Average and maximum `background_frame` duration.
- Average and maximum `app.window_logic` duration.
- UI callback count/rate and average/maximum UI construction duration.

Build the 0.1.3-debug release candidate with the existing low-resource experiments and GUI profiler:

```sh
cargo build --locked --release --features "nano-one-worker nano-ultra-low-resource nano-gui-profile"
```

Look for `gui_profile` in the application log. The counters describe native GUI callback activity; they do not measure GPU presentation time, compositor scheduling, backend thread CPU, or battery discharge. The profiler adds a small timing/logging cost, so use it to locate hot phases and callback frequency, then repeat the actual memory/CPU benchmark without the profiler feature. Do not treat callback rate alone as proof of an OS wakeup or GPU bottleneck.

## One-worker runtime experiment

An isolated A/B build is available on branch `nano/one-worker-experiment`.

Default build (two Tokio worker threads per account backend):

```sh
cargo build --locked --release
```

Experimental build (one Tokio worker thread per account backend):

```sh
cargo build --locked --release --features nano-one-worker
```

Use the same commit, machine, account, window state, warm-up period and measurement script. Fully exit the previous process before launching the other binary. Record process count as well as memory and CPU.

This feature changes only the backend runtime worker count. It does not replace the multi-thread Tokio scheduler, alter protocol logic, or change the default build. One worker may increase latency if CPU-heavy work competes with network tasks, so compare message responsiveness, synchronization, media transfers and calls before considering a default change.

Do not expect a large RAM reduction: the primary purpose is to measure the cost and sufficiency of one fewer runtime worker thread.

## Low-resource animation experiment

Branch: `nano/low-resource-animation`, based on `nano/one-worker-experiment`.

Build the one-worker comparison without the low-resource profile:

```sh
cargo build --locked --release --features nano-one-worker
```

Build the low-resource comparison:

```sh
cargo build --locked --release --features "nano-one-worker nano-low-resource"
```

The opt-in `nano-low-resource` feature makes three targeted changes:

- Runs at most one animation decoder at a time (two in the normal build), leaving more CPU headroom for the UI and backend on older processors.
- Reduces the resident animation texture budget from 450 frames to 120.
- Bounds both width and height of retained playback frames to 320 pixels. The normal build retains the existing full-height behaviour for tall portrait animations; the low-resource build scales those frames down while preserving aspect ratio and frame timing.

It deliberately keeps the 150-frame per-animation decode ceiling and does not change the WhatsApp protocol, archive, message history, or files on disk. Visible playing animations remain protected from eviction, so 120 is a cache budget, not a hard process-memory ceiling. A single decoder can also make simultaneous animation previews arrive more slowly.

Run the animation tests with the feature enabled:

```sh
cargo test --locked --features "nano-one-worker nano-low-resource" animation::
```

For a useful comparison, use the same account and window size, open a tall animated WebP/GIF and a normal landscape clip, then compare private bytes/working set, CPU, playback smoothness, and message responsiveness. Close the app between builds. Do not judge battery impact from CPU percentage alone; use the battery-discharge procedure above. The feature remains experimental until the measurements and media-quality checks agree.

## Ultra-low-resource animation experiment

Branch: `nano/ultra-low-resource`, based on `nano/low-resource-animation`.

Build the previous low-resource profile (320px maximum side, 120 resident frames):

```sh
cargo build --locked --release --features "nano-one-worker nano-low-resource"
```

Build the more aggressive profile (256px maximum side, 64 resident frames):

```sh
cargo build --locked --release --features "nano-one-worker nano-ultra-low-resource"
```

The `nano-ultra-low-resource` Cargo feature automatically includes `nano-low-resource`. It therefore keeps the single animation decoder and additionally:

- Limits retained animation/poster frame dimensions to 256×256 while preserving aspect ratio.
- Reduces the global resident animation-frame budget from 120 to 64.
- Keeps the existing maximum of 150 decoded frames per animation, preserving the timeline of longer animations rather than truncating them.

At 256×256 RGBA, 64 frames represent about 16 MiB of pixel data before texture-driver overhead. This is a cache target, not a hard memory ceiling: visible animations are protected from eviction and temporary decode buffers also consume memory. The lower resolution may soften large sticker previews; check text and fine details before preferring it.

Run the profile-specific regression test:

```sh
cargo test --locked --features "nano-one-worker nano-ultra-low-resource" animation::tests::animated_webp_posters_are_bounded_and_playback_obeys_the_build_profile
```

Then run the full animation test group:

```sh
cargo test --locked --features "nano-one-worker nano-ultra-low-resource" animation::
```

Compare it against the previous 320px/120-frame build using the same account, selected chat, UI size, window visibility, warm-up time, and media state. This profile is experimental and is not enabled by default.

## WhatsNano 0.1.4 — activity-aware resource retention

The 0.1.4 image cache policy releases assets according to recent use instead of retaining scrolled-away images indefinitely:

| Build profile | Recent off-screen image budget |
|---|---:|
| Normal | 96 |
| `nano-low-resource` | 48 |
| `nano-ultra-low-resource` | 24 |

Images drawn in the current frame are always protected, even when the number of visible images exceeds the profile budget. Images that leave the visible frame get a 30-second reuse window, then their egui image cache and texture are forgotten. A single delayed repaint is requested for the next expiry; the cache does not run a periodic polling timer while the window is otherwise idle.

This bounds the retained off-screen image set and reduces idle memory after browsing image-heavy chats. It is not a hard process-memory limit: visible images, in-flight decodes, fonts, wallpaper, driver allocations and backend state are separate.

### 0.1.4 build and test

Build the resource-conscious release candidate:

```powershell
cargo build --locked --release --features "nano-one-worker nano-ultra-low-resource"
```

Run the image-cache tests:

```powershell
cargo test --locked --features "nano-one-worker nano-ultra-low-resource" image_cache::
```

Run the animation regression tests too:

```powershell
cargo test --locked --features "nano-one-worker nano-ultra-low-resource" animation::
```

For the memory check, launch the same account, scroll through several image-heavy chats, return to one ordinary chat, wait for the 30-second cleanup window, then take the same private-bytes and working-set samples as the 0.1.3-debug baseline. Compare like-for-like release builds. The one-worker and ultra-low-resource features remain explicit build choices; this change does not silently change the default Tokio worker count or media resolution.

### Idle background work

The worker keeps its five-second connection-health and retry watchdog: this is intentionally unchanged so a dropped or silent connection is still detected promptly. Private read/unread state synchronization is now event-driven between those checks. A durable queue check is requested when the account connects or a read/unread state changes, and retries retain their existing backoff. Once both archive queues are confirmed empty, the worker skips further SQLite queue scans until new work or a reconnect occurs.


Poll-vote recovery is event-driven as well. The five-second watchdog no longer queries SQLite while the pending poll-vote queue is idle. Reconnects, new vote events, poll creation or history updates, and decryption completions wake the queue; the existing batch limit and reconnect retry behaviour remain in place. Database errors keep the queue armed for a later retry.

Favorite-chat synchronization now uses the same event-driven idle policy. The five-second maintenance tick skips favorite-list SQLite checks when no change is queued. Connections, local favorite changes, phone list updates, and completed sync operations wake it; failed operations retain the existing bounded retry backoff.
