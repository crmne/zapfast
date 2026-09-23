# Contributing to ZapFast

ZapFast is a small native WhatsApp client. Changes should improve the desktop
app without adding a browser engine, telemetry, a hosted backend, or another
protocol implementation.

## Before opening an issue

Search open and closed issues first. For a bug, use the bug form and include
the requested log and exact steps to reproduce it. Reports without enough
information to investigate may be closed.

For a feature, explain the user problem. Discuss large changes in an issue
before writing code. Existing code does not guarantee that a feature fits the
project.

Some boundaries come from WhatsApp or from upstream libraries:

- The protocol comes from [whatsapp-rust](https://github.com/oxidezap/whatsapp-rust).
  A capability it does not support is fixed upstream first, not reimplemented
  here.
- ZapFast will not embed a browser engine, add telemetry, or introduce a
  ZapFast-operated service. Features that send message content to a third
  party are out of scope.

Never post screenshots of real conversations, contact names, phone numbers,
keys, or QR codes. Crop a capture to the part that shows the problem and
redact everything personal in it.

Duplicate, out-of-scope, or incomplete issues may be closed with a short
explanation. A bug can be closed once its fix is on `main`, with the commit and
release status stated. Reopen the issue if it persists after updating.

## Design principles

1. **Native and fast.** Startup time, idle work, memory use, and binary size
   are product features. Keep the UI thread free of network and disk waits.
2. **Focused.** Prefer a complete, coherent workflow over a collection of
   settings, modes, and speculative features.
3. **Honest integrations.** Use whatsapp-rust for what it supports. Do not
   advertise a capability merely because a protobuf field exists for it.
4. **Cross-platform by default.** Linux, macOS, and Windows are supported
   products. Platform-specific code must be isolated and the other targets
   must keep compiling.
5. **Small dependency surface.** Reuse the standard library and existing
   crates where practical. A new dependency needs a concrete benefit worth its
   build time, binary size, maintenance, and security cost. Do not vendor or
   fork upstream crates such as egui.
6. **Private data.** The archive is personal data. Never log message contents,
   phone numbers, keys, or QR payloads.

## Pull requests

Keep each pull request to one change. A pull request that bundles unrelated
fixes or features will be closed with a request to split it. Explain why the
change belongs in ZapFast, what changed, and how you tested it. Avoid unrelated
formatting, refactors, generated prose, and large mechanical rewrites.

`main` has a linear history. Outside pull requests are squash-merged into one
focused commit with contributor credit; merge commits are not accepted. A
maintainer may push fixes to your branch before merging it.

The same rules apply to hand-written and AI-assisted changes. The author must
understand every line and answer review comments with specific reasoning.

Code changes should include tests for behaviour that can regress. User-visible
behaviour, settings, files, or network access must be documented in the same
pull request.

### Screenshots

Every pull request that changes what the app looks like needs before-and-after
screenshots or a short recording **in the pull request description**. Capture
them with the `demo` feature (`cargo run --features demo -- --demo`, or
`--demo-shot` for a headless capture) so they show only synthetic content, and
include light and dark themes when colours or layout change. Do not commit
screenshots, recordings, or other media to the repository.

### Checks

Run the same checks CI runs before submitting:

```sh
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
```

Linux needs the development packages listed in the README; `nix develop`
provides the complete development environment. When changing `Cargo.lock` or
`flake.nix`, also verify `nix build` on a Nix host or wait for the Nix CI job.
Passing CI is required, but does not replace review for correctness, product
fit, maintainability, or security.

`AGENTS.md` describes the architecture and conventions in more detail.

By contributing, you agree that your contribution is licensed under the
project's MIT License.
