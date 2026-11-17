//! Archive writes that commit together (#839): tests that batching files
//! the same rows, and a benchmark of what a first sync writes.
//!
//! The benchmark is ignored, so it never runs with the suite. Run it alone,
//! since the counters are the whole process's:
//!
//! ```sh
//! cargo test --lib history_write_bench -- --ignored --nocapture
//! ```
//!
//! `ZAPFAST_BENCH_CHATS`, `ZAPFAST_BENCH_MESSAGES` and
//! `ZAPFAST_BENCH_REMOVALS` change the size. All the data is synthetic:
//! made-up ids, text and thumbnails.

use super::*;

const ME_IN_GROUPS: &str = "15550000000@s.whatsapp.net";

fn setting(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// `write_bytes` (what reached the storage layer) and `wchar` (what was
/// handed to `write`) from `/proc/self/io`, where there is one.
fn io_counters() -> Option<(u64, u64)> {
    let text = std::fs::read_to_string("/proc/self/io").ok()?;
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(name)?.trim().parse().ok())
    };
    Some((field("write_bytes:")?, field("wchar:")?))
}

fn size(path: &Path) -> u64 {
    std::fs::metadata(path).map_or(0, |meta| meta.len())
}

fn mib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0)
}

/// A deterministic stand-in for random bytes.
fn filler(seed: usize, len: usize) -> Vec<u8> {
    let mut state = seed as u64 ^ 0x9e37_79b9_7f4a_7c15;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state as u8
        })
        .collect()
}

fn entry(
    chat: &str,
    n: usize,
    participant: Option<String>,
    from_me: bool,
    recipients: &[String],
) -> wa::HistorySyncMsg {
    let timestamp = 1_700_000_000 + n as u64 * 37;
    let message = {
        let mut proto = wa::Message::default();
        match n % 20 {
            // A photo with its preview, as most chats have now and then.
            0 | 1 => {
                proto.image_message = MessageField::some({
                    let mut image = wa::message::ImageMessage::default();
                    image.url = Some(format!("https://mmg.example.invalid/{n}"));
                    image.direct_path = Some(format!("/v/t62/{n}"));
                    image.mimetype = Some("image/jpeg".into());
                    image.media_key = Some(filler(n, 32));
                    image.file_sha256 = Some(filler(n + 1, 32));
                    image.file_enc_sha256 = Some(filler(n + 2, 32));
                    image.file_length = Some(180_000);
                    image.width = Some(1280);
                    image.height = Some(960);
                    image.jpeg_thumbnail = Some(filler(n + 3, 6_000));
                    image
                });
            }
            // A reply quoting an earlier message.
            2..=4 => {
                proto.extended_text_message = MessageField::some({
                    let mut text = wa::message::ExtendedTextMessage::default();
                    text.text = Some(format!("a reply number {n} with a few more words in it"));
                    text.context_info = MessageField::some({
                        let mut context = wa::ContextInfo::default();
                        context.stanza_id = Some(format!("M{}", n.saturating_sub(3)));
                        context.participant = Some(chat.to_owned());
                        context.quoted_message = MessageField::some({
                            let mut quoted = wa::Message::default();
                            quoted.conversation = Some("the message replied to".into());
                            quoted
                        });
                        context
                    });
                    text
                });
            }
            _ => {
                proto.conversation = Some(format!(
                    "synthetic message {n}: a line of ordinary chat text of ordinary length"
                ));
            }
        }
        proto
    };
    let mut proto = wa::HistorySyncMsg::default();
    proto.msg_order_id = Some(n as u64);
    proto.message = MessageField::some({
        let mut info = wa::WebMessageInfo::default();
        info.key = MessageField::some({
            let mut key = wa::MessageKey::default();
            key.remote_jid = Some(chat.into());
            key.from_me = Some(from_me);
            key.id = Some(format!("M{n}"));
            key.participant = participant;
            key
        });
        info.message = MessageField::some(message);
        info.message_timestamp = Some(timestamp);
        if !from_me {
            info.push_name = Some(format!("Person {}", n % 97));
        }
        info.status = Some(if from_me {
            wa::web_message_info::Status::READ
        } else {
            wa::web_message_info::Status::DELIVERY_ACK
        });
        info.user_receipt = recipients
            .iter()
            .map(|recipient| {
                let mut receipt = wa::UserReceipt::default();
                receipt.user_jid = recipient.clone();
                receipt.receipt_timestamp = Some(timestamp as i64 + 2);
                receipt.read_timestamp = Some(timestamp as i64 + 60);
                receipt
            })
            .collect();
        info
    });
    proto
}

/// A sync of `chats` chats holding `messages` messages between them, a
/// quarter of them groups, in chunks of `per_chunk` chats.
fn synthetic_history(chats: usize, messages: usize, per_chunk: usize) -> Vec<ParsedHistory> {
    // Some chats are busy and most are quiet, as on a real phone.
    let weights: Vec<usize> = (0..chats).map(|i| 1 + 40 / (1 + i % 50)).collect();
    let total: usize = weights.iter().sum();
    let mut n = 0;
    let mut parsed = Vec::new();
    for (i, weight) in weights.iter().enumerate() {
        let group = i % 4 == 0;
        let id = if group {
            format!("1203630{i:08}@g.us")
        } else {
            format!("4915{i:08}@s.whatsapp.net")
        };
        let members: Vec<String> = (0..if group { 25 } else { 0 })
            .map(|m| format!("4917{i:04}{m:04}@s.whatsapp.net"))
            .collect();
        let count = (messages * weight / total).max(1);
        let mut conversation = wa::Conversation::default();
        conversation.id = id.clone();
        conversation.name = Some(format!("Chat {i}"));
        conversation.messages = (0..count)
            .map(|k| {
                n += 1;
                let from_me = k % 3 == 0;
                let participant = (group && !from_me).then(|| members[k % members.len()].clone());
                let recipients: &[String] = if group && from_me { &members } else { &[] };
                entry(&id, n, participant, from_me, recipients)
            })
            .collect();
        // History lists newest first.
        conversation.messages.reverse();
        conversation.conversation_timestamp = Some(1_700_000_000 + n as u64 * 37);
        parsed.push(parse_conversation(conversation));
    }
    let mut chunks = Vec::new();
    let mut parsed = parsed.into_iter().peekable();
    while parsed.peek().is_some() {
        chunks.push(ParsedHistory {
            chats: parsed.by_ref().take(per_chunk).collect(),
            push_names: Vec::new(),
            lids: Vec::new(),
            stickers: Vec::new(),
        });
    }
    chunks
}

/// The phone's "delete for me" of `id`, as the runtime queues it.
fn deletion(worker: &Worker, chat: &str, id: &str) -> RuntimeEvent {
    RuntimeEvent::MessageRemoval {
        generation: worker.privacy_generation,
        event: Arc::new(wa_events::Event::DeleteMessageForMeUpdate(
            wa_events::DeleteMessageForMeUpdate::builder()
                .chat_jid(chat.parse().unwrap())
                .message_id(id.to_owned())
                .from_me(false)
                .timestamp(std::time::SystemTime::now().into())
                .action(Box::default())
                .from_full_sync(true)
                .build(),
        )),
    }
}

struct Measured {
    seconds: f64,
    write_bytes: u64,
    wchar: u64,
}

fn measure(work: impl FnOnce()) -> Measured {
    let before = io_counters();
    let started = Instant::now();
    work();
    let seconds = started.elapsed().as_secs_f64();
    let after = io_counters();
    let (write_bytes, wchar) = match (before, after) {
        (Some(before), Some(after)) => (after.0 - before.0, after.1 - before.1),
        _ => (0, 0),
    };
    Measured {
        seconds,
        write_bytes,
        wchar,
    }
}

#[test]
#[ignore = "a benchmark: run alone with --ignored --nocapture"]
fn history_write_bench() {
    let chats = setting("ZAPFAST_BENCH_CHATS", 2_000);
    let messages = setting("ZAPFAST_BENCH_MESSAGES", 40_000);
    let removals = setting("ZAPFAST_BENCH_REMOVALS", 5_000);
    let chunks = synthetic_history(chats, messages, 40);
    let stored: usize = chunks
        .iter()
        .flat_map(|chunk| &chunk.chats)
        .map(|chat| chat.messages.len())
        .sum();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("archive.db");
    let (mut worker, events, _commands, _wa) = super::receipt_tests::worker();
    worker.archive = Archive::open_encrypted_for_test(&path, &[7; 32]).unwrap();
    worker.me_pn = Some(ME_IN_GROUPS.to_owned());
    let chat_ids: Vec<String> = chunks
        .iter()
        .flat_map(|chunk| &chunk.chats)
        .map(|chat| chat.id.clone())
        .collect();

    let history = measure(|| {
        for chunk in chunks {
            worker.apply_history(chunk, true);
            while events.try_recv().is_ok() {}
        }
    });
    let after_history = (size(&path), size(&path.with_extension("db-wal")));

    // The phone sends recent history first and the full history after it,
    // so a first sync files many messages twice.
    let replay = synthetic_history(chats, messages, 40);
    let replayed = measure(|| {
        for chunk in replay {
            worker.apply_history(chunk, true);
            while events.try_recv().is_ok() {}
        }
    });

    // The phone's "deleted for me" list, mostly naming messages this
    // device never had, as the report's log shows. They arrive as events,
    // all queued at once, and go through the worker's event loop.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, mut waiting) = mpsc::unbounded_channel();
    for i in 0..removals {
        let chat = &chat_ids[i % chat_ids.len()];
        let _ = sender.send(deletion(&worker, chat, &format!("GONE{i}")));
    }
    drop(sender);
    let deletions = measure(|| {
        runtime.block_on(async {
            while let Some(event) = waiting.recv().await {
                worker.runtime_events(event, &mut waiting).await;
                while events.try_recv().is_ok() {}
            }
        });
    });

    let count: i64 = worker
        .archive
        .count_messages_for_test()
        .expect("count the messages");
    assert_eq!(count as usize, stored, "every message was stored");
    println!(
        "history: {chats} chats, {stored} messages in {:.2} s; write_bytes {:.1} MiB, wchar {:.1} MiB; archive.db {:.1} MiB, wal {:.1} MiB",
        history.seconds,
        mib(history.write_bytes),
        mib(history.wchar),
        mib(after_history.0),
        mib(after_history.1),
    );
    println!(
        "replay: the same history again in {:.2} s; write_bytes {:.1} MiB, wchar {:.1} MiB",
        replayed.seconds,
        mib(replayed.write_bytes),
        mib(replayed.wchar),
    );
    println!(
        "deletions: {removals} in {:.2} s; write_bytes {:.1} MiB, wchar {:.1} MiB",
        deletions.seconds,
        mib(deletions.write_bytes),
        mib(deletions.wchar),
    );
}

fn test_worker() -> (Worker, std::sync::mpsc::Receiver<Event>) {
    let (mut worker, events, _commands, _wa) = super::receipt_tests::worker();
    worker.me_pn = Some(ME_IN_GROUPS.to_owned());
    (worker, events)
}

/// A chunk committed at once files exactly the rows that committing each
/// write on its own did, a replay of it included.
#[test]
fn batched_history_files_what_unbatched_filing_did() {
    let (mut batched, _batched_events) = test_worker();
    let (mut unbatched, _unbatched_events) = test_worker();
    for _ in 0..2 {
        for chunk in synthetic_history(30, 900, 7) {
            batched.apply_history(chunk, true);
        }
        for chunk in synthetic_history(30, 900, 7) {
            unbatched.file_history(chunk, true);
        }
    }
    assert!(
        !batched.archive.in_batch_for_test(),
        "every chunk committed"
    );
    let filed = crate::archive::tests::dump(&batched.archive);
    let expected: usize = synthetic_history(30, 900, 7)
        .iter()
        .flat_map(|chunk| &chunk.chats)
        .map(|chat| chat.messages.len())
        .sum();
    let messages = filed.iter().filter(|row| row.starts_with("messages:"));
    assert_eq!(messages.count(), expected);
    assert!(filed.iter().any(|row| row.starts_with("group_receipts:")));
    assert_eq!(filed, crate::archive::tests::dump(&unbatched.archive));
}

/// One message that cannot be filed loses only itself, not the chunk.
#[test]
fn a_message_that_cannot_be_filed_leaves_the_rest_of_its_chunk() {
    let (mut worker, _events) = test_worker();
    crate::archive::tests::reject_message(&worker.archive, "M5");
    let chunks = synthetic_history(3, 30, 3);
    let total: usize = chunks
        .iter()
        .flat_map(|chunk| &chunk.chats)
        .map(|chat| chat.messages.len())
        .sum();
    for chunk in chunks {
        worker.apply_history(chunk, true);
    }
    assert!(!worker.archive.in_batch_for_test());
    let stored = worker.archive.count_messages_for_test().unwrap() as usize;
    assert_eq!(stored, total - 1);
}

/// Events already queued commit together, up to the batch's limit; the
/// rest wait for the next turn of the loop.
#[tokio::test]
async fn queued_events_commit_together_up_to_the_batch_limit() {
    let (mut worker, _events) = test_worker();
    let chat = "491500000001@s.whatsapp.net";
    let (sender, mut waiting) = mpsc::unbounded_channel();
    let total = EVENT_BATCH + 10;
    for i in 0..total {
        sender
            .send(deletion(&worker, chat, &format!("GONE{i}")))
            .unwrap();
    }
    let first = waiting.recv().await.unwrap();
    worker.runtime_events(first, &mut waiting).await;
    assert!(!worker.archive.in_batch_for_test(), "the batch committed");
    let left = waiting.len();
    assert!(left >= 10, "a batch takes at most {EVENT_BATCH} events");
    for i in 0..total {
        assert_eq!(
            worker
                .archive
                .message_removed(chat, &format!("GONE{i}"))
                .unwrap(),
            i < total - left,
            "deletion {i}"
        );
    }
}
