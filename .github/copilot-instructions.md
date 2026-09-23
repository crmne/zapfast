# Copilot instructions

Read and follow `AGENTS.md` and `CONTRIBUTING.md` before reviewing or changing
this repository. `AGENTS.md` is the canonical architecture, product-boundary,
privacy, testing, and release guide. Keep changes narrowly scoped and preserve
existing behavior unless the task explicitly changes it.

## Pull request scope and evidence

Check these first, and report each failure as a blocker at the top of the
review, before any line comments:

- One concern per pull request. When a pull request bundles unrelated fixes or
  features, name the separate changes and ask for one pull request each. Do
  not review the rest in depth until it is split.
- Start every review with `User-visible UI impact: none` or a list of the
  visible changes. Treat changes to navigation, control placement, menus,
  panel sizing, spacing, or visual hierarchy as an interface change even when
  the code is correct.
- Any visible change needs before-and-after screenshots or a recording in the
  pull request description, captured with the `demo` feature (synthetic data,
  never real chats), in light and dark themes when colours or layout change.
  Ask for them when they are missing.
- Screenshots, recordings, and other media must not be committed to the
  repository. Flag any added image or video file that is not an app asset.
- Do not vendor, fork, or patch upstream crates (egui, epaint, whatsapp-rust)
  inside this repository. Changes those crates need go upstream.
- Flag any feature that sends message content, audio, or contacts to a third
  party. That is outside the product boundaries.

## Review priorities

- Treat privacy and local data integrity as release blockers. Never log message
  contents, phone numbers, device keys, authorization material, or QR payloads.
  Preserve the linked-device session, the message archive, raw attachment
  protobufs, and backward compatibility of settings and state files.
- Keep the UI/runtime boundary intact. Views in `src/ui/` draw and emit
  `model::Action`s, `src/app.rs` applies them after drawing, and WhatsApp or
  other blocking work runs through `Command` and `Event` on the backend
  runtime. Every backend event that affects the interface must wake the window.
- Use whatsapp-rust for protocol behavior. Do not treat protobuf fields as
  supported features, reimplement protocol pieces locally, or imply that an
  unsupported WhatsApp capability works.
- Keep protobufs out of `src/ui/` and `src/model.rs`. Translate them in the
  backend, canonicalize every arriving `Jid` through `Worker::canonical`, and
  retain raw messages where attachment recovery depends on their keys.
- Check optimistic and asynchronous state carefully. A delayed backend answer
  must not undo a newer action the person already sees.
- Keep Linux, macOS, and Windows compiling. Isolate platform behavior with
  target-specific modules or `cfg` blocks and call out platform coverage
  accurately.
- Route text that can contain emoji through the existing rich-text and markup
  paths. Preserve selectable transcript behavior, nested click targets, and
  right-aligned bubble layout rules described in `AGENTS.md`.
- For visual changes, use the deterministic `demo` feature and inspect the
  affected screens in representative sizes and both themes. Do not accept an
  interface redesign without explicit maintainer approval of its visual scope.
- Prefer existing dependencies. Flag new crates, changes to network access,
  storage formats, permissions, or release packaging for explicit scrutiny.
- Follow the trunk-based branch policy in `AGENTS.md`. Maintainer and agent
  work goes directly to a linear `main`; do not create a branch unless the
  maintainer explicitly requests one. Every ordinary release tag, including a
  prerelease, must already be reachable from `origin/main`.
- Require focused regression tests and the full checks from `AGENTS.md` for
  code changes. Do not weaken a lint or test to make a change pass.

## Review communication

After the scope and evidence checks, lead with concrete, actionable defects
introduced by the change. Distinguish confirmed bugs from questions, avoid
speculative redesigns and adjacent refactors, and do not claim a platform was
tested when it was only inspected or compiled. CI passing is necessary but not proof that a change is correct. Never
approve, close, or merge a pull request; the maintainer decides. Never use em
dashes in repository-facing prose.
