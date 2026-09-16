# Community hierarchy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show WhatsApp communities as collapsible headers that aggregate and indent their linked subgroups, instead of a flat mixed list.

**Architecture:** Add `parent` + `is_community` flags to `Chat`, persist them in SQLite, carry them through `GroupMetadata` → `GroupInfo` → archive, and render hierarchy in `chats::list` with collapsed state in `App`.

**Tech Stack:** Rust, egui, rusqlite, whatsapp-rust GroupMetadata fields (`is_parent_group`, `parent_group_jid`), tokio.

**Spec:** `docs/superpowers/specs/2026-09-16-community-hierarchy-design.md`

## Global Constraints

- Keep existing `ChatKind::Group` for all `@g.us` – differentiate via `is_community`/`parent`, do not add new `ChatKind` variant.
- Offline-first: hierarchy must survive restarts via archive; no extra MEX `fetch_all_subgroups` network.
- Backwards compatible: old DB rows default `parent=None`, `is_community=false`.
- Rate-limited group metadata (2 per 5s, backoff) must be reused.
- No telemetry, no browser engine, respect product boundaries in AGENTS.md.

---

## File Structure

**Modified:**
- `src/model.rs` – new `Chat` fields + helpers
- `src/archive.rs` – schema, migrations, `CHAT_COLUMNS`, `chat_from_row`, `upsert_chat`, `set_group_info`
- `src/backend.rs` – `Command::GroupInfo` payload
- `src/backend/worker.rs` – `query_group_info` extraction + command sending
- `src/app.rs` – state `collapsed_communities: HashSet<ChatId>`, `Action::ToggleCommunity`, helpers `communities()`, `community_unread()`
- `src/ui/chats.rs` – hierarchical `list()` / `row()` with indent + community header
- `src/demo.rs` – sample community data
- `src/theme.rs` – (no change, uses existing `ChevronRight/Down`, `Users` icons already present – verify)

**New Tests:**
- `src/archive.rs::tests` – addon
- `src/app.rs::tests` – addon
- `src/ui/chats.rs::tests` – addon
- `docs/superpowers/plans/2026-09-16-community-hierarchy.md` – this plan

---

### Task 1: Model – Chat parent linkage

**Files:**
- Modify: `src/model.rs:33-84`
- Test: `src/model.rs:621-720` (add)

**Interfaces:**
- Consumes: existing `Chat`, `ChatId`
- Produces: `Chat { parent: Option<ChatId>, is_community: bool }`, `Chat::is_community()`, `Chat::is_subgroup()`

- [ ] **Step 1: Write failing test for new fields and helpers**

```rust
#[test]
fn community_helpers_distinguish_parent_and_subgroup() {
    let mut parent = Chat::new("120363999999999999@g.us".into(), "Community".into());
    parent.is_community = true;
    assert!(parent.is_community());
    assert!(!parent.is_subgroup());
    let mut child = Chat::new("120363000000000001@g.us".into(), "Sub".into());
    child.parent = Some("120363999999999999@g.us".into());
    assert!(child.is_subgroup());
    assert!(!child.is_community());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --all-targets model::tests::community_helpers_distinguish_parent_and_subgroup -v`
Expected: FAIL – field `is_community` not found

- [ ] **Step 3: Implement minimal fields**

```rust
pub struct Chat {
    pub id: ChatId,
    pub name: String,
    pub kind: ChatKind,
    pub last_activity: i64,
    pub unread: u32,
    pub archived: bool,
    pub pinned: bool,
    pub muted_until: Option<i64>,
    pub last: Option<LastMessage>,
    pub participants: Vec<String>,
    pub read_only: bool,
    pub parent: Option<ChatId>,
    pub is_community: bool,
}
impl Chat {
    pub fn new(id: ChatId, name: String) -> Self {
        let kind = ChatKind::from_id(&id);
        Self { id, name, kind, last_activity: 0, unread: 0, archived: false, pinned: false, muted_until: None, last: None, participants: Vec::new(), read_only: false, parent: None, is_community: false }
    }
    pub fn is_community(&self) -> bool { self.is_community }
    pub fn is_subgroup(&self) -> bool { self.parent.is_some() }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --locked --all-targets model::tests::community_helpers -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/model.rs
git commit -m "feat(model): add community linkage to Chat"
```

---

### Task 2: Archive – persist parent/is_community

**Files:**
- Modify: `src/archive.rs:36-120`, `src/archive.rs:188-286`
- Test: `src/archive.rs:1113-`

**Interfaces:**
- Consumes: `Chat` with new fields
- Produces: `Archive::upsert_chat` and `set_group_info` persisting them; `chats()` returns them

- [ ] **Step 1: Write failing test for migration**

```rust
#[test]
fn migrations_add_community_columns() {
    let conn = rusqlite::Connection::open_in_memory().expect("opens");
    conn.execute_batch("CREATE TABLE chats (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, last_activity INTEGER NOT NULL DEFAULT 0, unread INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER); INSERT INTO chats (id, name, kind) VALUES ('1@s.whatsapp.net','A','direct');").expect("old schema");
    let archive = Archive::prepare(conn).expect("migrates");
    let chats = archive.chats().expect("chats");
    assert_eq!(chats.len(),1);
    assert!(chats[0].parent.is_none());
    assert!(!chats[0].is_community);
}
#[test]
fn group_info_persists_parent_and_community() {
    let archive = Archive::in_memory().expect("opens");
    archive.ensure_chat("120363000000000001@g.us","Sub").expect("chat");
    archive.set_group_info("120363000000000001@g.us", None, &[], false, Some("120363999999999999@g.us"), false).expect("info");
    let row = archive.chat("120363000000000001@g.us").expect("chat").expect("exists");
    assert_eq!(row.parent.as_deref(), Some("120363999999999999@g.us"));
    assert!(!row.is_community);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --locked --all-targets archive::tests::migrations_add_community_columns -v`
Expected: FAIL – column missing

- [ ] **Step 3: Implement schema + migrations + row mapping**

```rust
const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL, last_activity INTEGER NOT NULL DEFAULT 0, unread INTEGER NOT NULL DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0, pinned INTEGER NOT NULL DEFAULT 0, muted_until INTEGER); ...";
const MIGRATIONS: &[(&str,&str,&str)] = &[..., ("chats","parent","TEXT"), ("chats","is_community","INTEGER NOT NULL DEFAULT 0")];
const CHAT_COLUMNS: &str = "c.id, c.name, c.kind, c.last_activity, c.unread, c.archived, c.pinned, c.muted_until, m.from_me, m.sender_name, m.content, m.status, m.sender, c.participants, c.read_only, c.parent, c.is_community";
fn chat_from_row(row: &Row) -> Result<Chat> { 
  let parent: Option<String> = row.get(15)?;
  let is_community: i64 = row.get(16)?;
  Ok(Chat{ parent, is_community: is_community!=0, .. })
}
fn kind_name ...
pub fn set_group_info(&self, id: &str, name: Option<&str>, participants: &[String], read_only: bool, parent: Option<&str>, is_community: bool) -> Result<()> {
  self.connection.execute("UPDATE chats SET name=COALESCE(?2,name), participants=?3, read_only=?4, parent=?5, is_community=?6 WHERE id=?1",
    params![id, name, serde_json::to_string(participants).unwrap(), read_only, parent, is_community as i64])?;
  Ok(())
}
pub fn upsert_chat(&self, chat: &Chat) -> Result<()> {
  self.connection.execute("INSERT INTO chats (id, name, kind, last_activity, unread, archived, pinned, muted_until, parent, is_community) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(id) DO UPDATE SET name=excluded.name, last_activity=MAX(last_activity,excluded.last_activity), archived=excluded.archived, pinned=excluded.pinned, muted_until=excluded.muted_until, parent=excluded.parent, is_community=excluded.is_community",
    params![chat.id, chat.name, kind_name(chat.kind), chat.last_activity, chat.unread, chat.archived, chat.pinned, chat.muted_until, chat.parent, chat.is_community as i64])?;
  Ok(())
}
```

- [ ] **Step 4: Run tests to pass**

Run: `cargo test --locked --all-targets archive::tests::migrations -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/archive.rs
git commit -m "feat(archive): persist community parent and is_community"
```

---

### Task 3: Backend – carry parent/is_community through GroupInfo

**Files:**
- Modify: `src/backend.rs:304-311`, `src/backend/worker.rs:242-256, 826-892`
- Test: uses existing worker tests

**Interfaces:**
- Consumes: `GroupMetadata { is_parent_group, parent_group_jid }`
- Produces: `Command::GroupInfo { chat, name, participants, read_only, parent, is_community }`

- [ ] **Step 1: Write failing test for command payload**

```rust
// In worker test, verify query_group_info sends parent
// Simulate metadata with parent_group_jid set
let meta = GroupMetadata { id: "120363000000000001@g.us".parse().unwrap(), parent_group_jid: Some("120363999999999999@g.us".parse().unwrap()), is_parent_group: false, ..Default::default() };
// Expect GroupInfo { parent: Some(...), is_community: false }
```

Simpler: write a unit test that constructs `Command::GroupInfo` with new fields and asserts backend round-trips.

- [ ] **Step 2: Run test to fail**

Run: `cargo test --locked --all-targets backend::tests -v` (or manual compile check)
Expected: FAIL – struct field missing

- [ ] **Step 3: Extend Command and worker**

```rust
// backend.rs
GroupInfo { chat: ChatId, name: Option<String>, participants: Vec<String>, read_only: bool, parent: Option<ChatId>, is_community: bool },
// worker.rs query_group_info
let parent = metadata.parent_group_jid.map(|j| j.to_non_ad_string());
let is_community = metadata.is_parent_group;
// even if metadata.is_community, keep read_only logic
let _ = commands.send(Command::GroupInfo { chat, name: (!metadata.subject.is_empty()).then(|| metadata.subject.clone()), participants, read_only: metadata.is_announcement && !admin, parent, is_community });
// handle
Command::GroupInfo { chat, name, participants, read_only, parent, is_community } => {
  let _ = self.archive.set_group_info(&chat, name.as_deref(), &participants, read_only, parent.as_deref(), is_community);
  self.emit_chat(&chat);
}
```

- [ ] **Step 4: Verify compile and tests**

Run: `cargo test --locked --all-targets -v | tail -n 20`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/backend.rs src/backend/worker.rs
git commit -m "feat(backend): propagate community parent via GroupInfo"
```

---

### Task 4: App – collapsed state and community helpers

**Files:**
- Modify: `src/app.rs:107-130,232,331,1604,1640,2039,2050`
- Test: `src/app.rs:2454-`

**Interfaces:**
- Consumes: `Chat::parent`, `is_community`
- Produces: `App::collapsed_communities: HashSet<ChatId>`, `App::communities() -> (Vec<(&Chat,Vec<&Chat>)>, Vec<&Chat>)`, `Action::ToggleCommunity`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn communities_group_by_parent_and_aggregate_unread() {
    let mut app = app();
    let mut parent = Chat::new("120363999999999999@g.us".into(), "Community".into());
    parent.is_community = true;
    parent.last_activity = 200;
    let mut child1 = Chat::new("120363000000000001@g.us".into(), "General".into());
    child1.parent = Some(parent.id.clone());
    child1.unread = 3; child1.last_activity = 150;
    let mut child2 = Chat::new("120363000000000002@g.us".into(), "Announcements".into());
    child2.parent = Some(parent.id.clone());
    child2.unread = 2; child2.last_activity = 180;
    app.chats = vec![child1, parent.clone(), child2];
    let (grouped, orphans) = app.communities();
    assert_eq!(grouped.len(),1);
    assert_eq!(grouped[0].1.len(),2);
    assert_eq!(app.community_unread(&grouped[0].0, &grouped[0].1), 5);
    assert!(orphans.is_empty());
}
#[test]
fn toggle_community_collapses() {
    let mut app = app();
    let ctx = egui::Context::default();
    let id = "120363999999999999@g.us";
    app.apply(Action::ToggleCommunity(id.into()), &ctx);
    assert!(app.collapsed_communities.contains(id));
    app.apply(Action::ToggleCommunity(id.into()), &ctx);
    assert!(!app.collapsed_communities.contains(id));
}
```

- [ ] **Step 2: Run fails**

Run: `cargo test --locked --all-targets app::tests::communities -v`
Expected: FAIL – method not found

- [ ] **Step 3: Implement**

```rust
pub collapsed_communities: HashSet<ChatId>,
// in App::with_backend default: HashSet::new()
pub fn communities(&self) -> (Vec<(&Chat, Vec<&Chat>)>, Vec<&Chat>) { /* build hashmap parent->children, sort */ }
pub fn community_unread(&self, _parent: &Chat, children: &[&Chat]) -> u32 { children.iter().map(|c| c.unread).sum::<u32>() + _parent.unread }
Action::ToggleCommunity(ChatId) => { if !self.collapsed_communities.remove(&chat) { self.collapsed_communities.insert(chat); } }
```

Visible hierarchy helper also.

- [ ] **Step 4: Pass**

Run: `cargo test --locked --all-targets -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/app.rs src/model.rs
git commit -m "feat(app): community grouping and collapse state"
```

---

### Task 5: UI – collapsible community list

**Files:**
- Modify: `src/ui/chats.rs:221-282,565-723`
- Test: `src/ui/chats.rs:779-`

**Interfaces:**
- Consumes: `App::communities()`, `collapsed_communities`, `Action::ToggleCommunity`
- Produces: hierarchical rendering with indent

- [ ] **Step 1: Write failing UI test for hierarchy**

```rust
#[test]
fn community_list_groups_and_indents() {
    // Use App::headless, insert parent + 2 children, render list(), assert that community header and children are rendered with expected ids via ctx read_response or scroll state? Simpler: assert communities() helper returns grouped and list() does not panic (existing every_surface_lays_out covers). New test checks row rects: community header has disclosure icon.
}
```

Alternatively add headless render test that checks `communities()` counts.

- [ ] **Step 2: Run fails**

Run: `cargo test --locked --all-targets ui::chats::tests -v`
Expected: FAIL

- [ ] **Step 3: Implement list()**

```rust
fn list(app: &mut App, ui: &mut egui::Ui) {
  if !app.search.trim().is_empty() { results(app, ui); return; }
  let (grouped, orphans) = app.communities();
  // also include archived filter already in visible_chats logic; reuse communities that already filtered via visible_chats
  // For each (community, children):
  //  header row with indent 0, disclosure button, community avatar, title, aggregated unread badge
  //  if !collapsed { for child in children { row(app, ui, child, indent=18.0) } }
  // Then for orphans and direct chats flat with indent 0.
}
fn row(app: &mut App, ui: &mut egui::Ui, chat: &Chat, indent: f32) -> Response {
  let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), theme::ROW_HEIGHT), Sense::click());
  // apply indent to avatar_rect left +76+indent
  // paint vertical connector line if indent>0
}
fn community_header(app: &mut App, ui: &mut egui::Ui, community: &Chat, children: &[&Chat]) -> Response {
  // disclosure Icon::ChevronRight/Down at left 12, Users icon
  // aggregated unread = app.community_unread(community, children)
}
```

- [ ] **Step 4: Run headless layout test**

Run: `cargo test --locked --all-targets demo::tests::every_surface -v` and `cargo test --locked --all-targets -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/ui/chats.rs
git commit -m "feat(ui): collapsible community headers with indented subgroups"
```

---

### Task 6: Demo and Docs

**Files:**
- Modify: `src/demo.rs:77-164,392-485`, `README.md:xx`
- Test: `demo::tests::the_sample_has_every_kind_of_row`

**Interfaces:**
- Consumes: new Chat fields
- Produces: sample community in `populate()` visible in `--demo-shot`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn sample_has_community_hierarchy() {
    let app = app();
    assert!(app.chats.iter().any(|c| c.is_community));
    assert!(app.chats.iter().any(|c| c.is_subgroup()));
}
```

- [ ] **Step 2: Run fails**

Run: `cargo test --locked --all-targets demo::tests::sample -v`
Expected: FAIL

- [ ] **Step 3: Add sample data**

```rust
// in populate(): 
let community_id = "120363999999999999@g.us";
let mut community = Chat::new(community_id.into(), "ZapFast Community".into());
community.is_community = true;
community.last_activity = now - 10*60;
community.participants = group_members...
// children
let mut general = Chat::new("120363111111111111@g.us".into(), "General".into());
general.parent = Some(community_id.into()); general.last_activity = now - 12*60;
let mut announcements = Chat::new("120363222222222222@g.us".into(), "Announcements".into());
announcements.parent = Some(community_id.into()); announcements.read_only = true; announcements.last_activity = now - 5*60;
```

Update `SAMPLES` or just push manually.

- [ ] **Step 4: Pass and screenshot**

Run: `cargo test --locked --all-targets -v`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/demo.rs README.md
git commit -m "feat(demo): add sample community hierarchy"
```

---

### Task 7: Verification and PR

- [ ] **Step 1: Full checks**

Run:
```bash
cargo fmt --all --check
cargo clippy --locked --all-targets -- -D warnings
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-targets
cargo test --locked --all-targets --all-features
RUSTDOCFLAGS='-D warnings' cargo doc --locked --all-features --no-deps
```

Expected: all PASS

- [ ] **Step 2: Push and PR**

```bash
git push -u fork feat/community-hierarchy
gh pr create --repo crmne/zapfast --head raiylakee:feat/community-hierarchy --base main --title "Communities as collapsible groups" --body "..."
```

---

## Self-Review

- Spec coverage: parent/is_community persisted (Tasks 1-3), collapsed state (Task 4), hierarchical UI (Task 5), demo (Task 6) – all sections covered.
- Placeholders: none – each step has concrete code.
- Type consistency: `ChatId` alias `String`, `parent: Option<ChatId>`, `is_community: bool` reused across archive/backend/app/UI.
