# Community hierarchy – design spec

Date: 2026-09-16
Status: approved (bounded → upgraded to architectural)
Feature: make communities distinct from flat chats; show hierarchy like official WhatsApp (community → channels/groups).

## Summary
WhatsApp communities are parent groups (`is_parent_group`) that own linked subgroups (`parent_group_jid`). ZapFast currently treats every `@g.us` id as `ChatKind::Group` with no parent linkage, so communities and their subgroups appear as unrelated flat rows sorted by `last_activity`. This spec adds a minimal parent linkage to `Chat` and a collapsible community header in the chat list, reusing existing group-metadata fetch.

## Goals
- Communities appear as collapsible headers (`Icon::Users` + name + Disclosure) aggregating unread counts of children; subgroups are indented underneath.
- Tapping the header opens the community announcement chat (the parent group itself); tapping a child opens that subgroup.
- Offline-capable: hierarchy survives restarts via archive persistence.
- No extra network beyond existing `Groups::get_metadata` rate-limited flow.
- Backward compatible with old DBs (missing columns default).

## Non-goals
- Community creation / linking UI (defer).
- Server-side mutations (link/unlink) – read-only grouping.
- Separate `Community` table or MEX `fetch_all_subgroups` polling (future if needed).
- Changing `ChatKind` enum to new variant – keep `@g.us` as Group and differentiate via flags.

## Architecture

### Model (`src/model.rs`)
- `Chat` adds:
  ```rust
  pub parent: Option<ChatId>,      // parent community id
  pub is_community: bool,          // is_parent_group
  ```
- Helpers:
  - `is_community() -> bool`
  - `is_subgroup() -> bool { parent.is_some() }`
- `is_group()` stays true for both parent and subgroups (all `@g.us`). `ChatKind` unchanged.

### Archive (`src/archive.rs`)
- Schema `chats` adds `parent TEXT` and `is_community INTEGER NOT NULL DEFAULT 0`.
- `MIGRATIONS` entries: `("chats","parent","TEXT")`, `("chats","is_community","INTEGER NOT NULL DEFAULT 0")`.
- `CHAT_COLUMNS` extended, `chat_from_row` reads 2 extra columns with defaults for old rows.
- `upsert_chat` includes new columns in insert and update (`parent = excluded.parent, is_community = excluded.is_community` where sensible).
- `set_group_info(id, name, participants, read_only, parent, is_community)` updates `parent`, `is_community` alongside existing fields.
- `kind_from_name` / `kind_name` unchanged.

### Backend (`src/backend.rs`, `src/backend/worker.rs`)
- `Command::GroupInfo` extended: `parent: Option<ChatId>, is_community: bool`.
- `Worker::query_group_info` now extracts:
  ```rust
  let parent = metadata.parent_group_jid.map(|j| j.to_non_ad_string());
  let is_community = metadata.is_parent_group;
  ```
  and sends `Command::GroupInfo { chat, name, participants, read_only, parent, is_community }`.
- `Worker::handle_command(Command::GroupInfo)` forwards to `archive.set_group_info(...)` and emits `ChatUpdated`.
- Existing backoff / `GroupInfoFailed` handling unchanged; orphan subgroups (parent not yet known) remain flat until parent metadata arrives, then regroup on next `emit_chats`.

### App (`src/app.rs`)
- State: `collapsed_communities: HashSet<ChatId>` (in-memory; optionally persisted via `Settings::collapsed_communities: HashSet<String>` later).
- New action: `Action::ToggleCommunity(ChatId)`.
- Helpers:
  - `fn communities(&self) -> Vec<(&Chat, Vec<&Chat>)>` – builds parent → children map from `self.chats` where `parent.is_some()` and parent exists; subgroups sorted by `last_activity` desc; communities sorted by `last_activity`; orphans returned separately.
  - `fn community_unread(&self, community: &Chat, children: &[&Chat]) -> u32` – sum.
- `visible_chats` stays for search path; new `visible_hierarchy` used by `chats::list` when search empty.

### UI (`src/ui/chats.rs`, `src/theme.rs`, `src/ui/widgets.rs`)
- `list()` switches: when search empty, builds hierarchical rows:
  - For each `(community, children)`:
    - Header row: height `ROW_HEIGHT`, left indent 0, disclosure icon `ChevronDown` (expanded) / `ChevronRight` (collapsed), community avatar, title, unread badge (aggregated), mute/pin icons. Click on header → `OpenChat(community.id)` plus `ToggleCommunity` on disclosure hit. Header has distinct `Icon::Users` badge or community color.
    - If not collapsed, for each child: `row(app, ui, child, indent=16.0)` – indented, smaller avatar? Keep 48 but x offset +14, left line color `palette.outline` vertical connector.
  - Then orphan groups and direct chats flat as before, after communities.
- `row()` gains `indent: f32` param; used to offset `avatar_rect` and `left`.
- Search `results()` still flat but subgroup hit shows community prefix: `"Community / Subgroup"`.
- Archived count still flat; community archived handling: if parent archived, children inherit archived filter? For now parent's `archived` controls visibility; children follow same `show_archived` flag independently (like official: archiving community archives announcement but not subgroups).
- New test: `bubble_hit_rects` etc remain; add `community_hierarchy_orders_and_collapses` headless test.

### Demo (`src/demo.rs`)
- Add sample community: parent `120363999999999999@g.us` ("ZapFast Community") with `is_community=true`, children `120363000000000001@g.us` ("Announcements", read_only) and `120363000000000002@g.us` ("General", General chat) – both `parent = Some(parent_id)`. Provide varied timestamps to show ordering.

## Data flow
1. History sync ingests chats → `archive.ensure_chat` + `request_group_info`.
2. `pump_group_info` rate-limits (2 per 5s) → `Groups::get_metadata` → `Command::GroupInfo` includes parent/is_community.
3. `archive.set_group_info` persists; `emit_chat` → `App` updates `chats` vec.
4. UI reads `App::communities()` each frame, renders hierarchy; collapse state toggled via `Action::ToggleCommunity`.

## Error handling
- Migration: `PRAGMA table_info` check before `ALTER TABLE`, no data loss.
- Old DB: `chat_from_row` treats NULL `parent` as `None`, NULL `is_community` as false.
- Parent missing: child shown as normal group until parent arrives.
- Rate limit: reuse existing `group_info_tries` backoff (30s doubling, 7 tries) and final stops on `item-not-found/forbidden/not-authorized`.

## Testing
- `cargo fmt`, `cargo clippy -- -D warnings`, `cargo test --all-targets (+--all-features)`, `RUSTDOCFLAGS='-D warnings' cargo doc`.
- New unit tests: `archive::tests::migrations_add_community_columns`, `app::tests::communities_group_by_parent`, `ui::chats::tests::collapsible_community_renders`.
- Demo headless `every_surface_lays_out` covers new rows with `--demo-shot community` flag.

## Release notes
User-visible: communities now appear as collapsible headers with indented subgroups, like official WhatsApp, instead of flat mixed list.

## Spec self-review
- No placeholders – all fields named, defaults explicit.
- Internal consistency checked: `is_community` vs `parent` orthogonal, `read_only` stays separate.
- Scope is single plan: model+archive+backend+UI hierarchy, no creation/linking UI.
- Ambiguity resolved: collapse is per-community header, tapping header opens parent chat (announcement), not just toggle; toggle only on disclosure area.
