# Performance baseline

This document defines a repeatable, privacy-preserving baseline for WhatsApp Nano (the ZapFast fork).

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
