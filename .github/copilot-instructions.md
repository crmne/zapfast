# Copilot instructions

Read and follow `AGENTS.md` before reviewing or changing this repository. It
is the canonical architecture, product-boundary, privacy, testing, and release
guide. Keep changes narrowly scoped and preserve existing behavior unless the
task explicitly changes it.

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

Lead with concrete, actionable defects introduced by the change. Distinguish
confirmed bugs from questions, avoid speculative redesigns and adjacent
refactors, and do not claim a platform was tested when it was only inspected or
compiled. Never use em dashes in repository-facing prose.
