use super::*;

fn dispatch(session: &mut Session, value: Value) -> Value {
    match session.dispatch(&serde_json::to_vec(&value).unwrap()) {
        Dispatch::Reply(value) => value,
        _ => panic!("expected protocol response"),
    }
}

#[test]
fn protocol_lifecycle_and_read_only_tools() {
    let mut session = Session::default();
    assert_eq!(
        dispatch(
            &mut session,
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
        )["error"]["code"],
        -32002
    );
    let init = dispatch(
        &mut session,
        json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
    );
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(matches!(
        session.dispatch(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        Dispatch::Silent
    ));
    let listed = dispatch(
        &mut session,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}),
    );
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    assert!(
        tools
            .iter()
            .all(|tool| tool["annotations"]["readOnlyHint"] == true
                && tool["annotations"]["openWorldHint"] == false)
    );
    assert_eq!(
        dispatch(
            &mut session,
            json!({"jsonrpc":"2.0","id":4,"method":"ping"})
        )["result"],
        json!({})
    );
    assert_eq!(
        dispatch(
            &mut session,
            json!({"jsonrpc":"2.0","id":5,"method":"send_message"})
        )["error"]["code"],
        -32601
    );
    match session.dispatch(
        br#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_chats"}}"#,
    ) {
        Dispatch::Query(id, Query::Chats { limit }) => {
            assert_eq!(id, 6);
            assert_eq!(limit, 20);
        }
        _ => panic!("expected bounded query"),
    }
}

#[test]
fn rejects_invalid_protocol_and_bounds() {
    let mut session = Session::default();
    for (bytes, code) in [
        (b"{".as_slice(), -32700),
        (b"[]".as_slice(), -32600),
        (
            br#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#.as_slice(),
            -32600,
        ),
    ] {
        match session.dispatch(bytes) {
            Dispatch::Reply(value) => assert_eq!(value["error"]["code"], code),
            _ => panic!("expected error"),
        }
    }
    assert!(matches!(
        session.dispatch(&vec![b' '; MAX_FRAME + 1]),
        Dispatch::Reply(_)
    ));
    for limit in [json!(0), json!(51), json!(-1), json!(1.5), json!("5")] {
        assert!(parse_query(&json!({"name":"list_chats","arguments":{"limit":limit}})).is_err());
    }
    for args in [
        json!({"query":""}),
        json!({"query":"x".repeat(257)}),
        json!({"query":"x\n"}),
        json!({"query":"ok", "sql":"DROP TABLE messages"}),
    ] {
        assert!(parse_query(&json!({"name":"search_messages","arguments":args})).is_err());
    }
    assert!(parse_query(&json!({"name":"send_message","arguments":{}})).is_err());
    assert!(
        parse_query(
            &json!({"name":"get_recent_messages","arguments":{"chat_id":"chat","limit":50}})
        )
        .is_ok()
    );
}

#[test]
fn archive_results_are_bounded_and_allowlisted() {
    let archive = Archive::in_memory().unwrap();
    for i in 0..60 {
        let mut chat = crate::model::Chat::new(format!("{i}@g.us"), format!("Chat {i}"));
        chat.last_activity = i;
        archive.upsert_chat(&chat).unwrap();
    }
    assert_eq!(archive.mcp_chats(usize::MAX).unwrap().len(), MAX_ROWS);
    assert_eq!(archive.mcp_chats(0).unwrap().len(), 0);
    let value = query_archive(&archive, &Query::Chats { limit: 2 }).unwrap();
    assert_eq!(value["chats"].as_array().unwrap().len(), 2);
    assert_eq!(value["chats"][0]["id"], "59@g.us");
    assert_eq!(value["chats"][0].as_object().unwrap().len(), 4);
    let message = Message {
        id: "message".into(),
        chat: "chat".into(),
        sender: "sender".into(),
        sender_name: Some("name\u{001b}".into()),
        from_me: false,
        timestamp: 123,
        content: Content::Contact {
            display_name: "Visible".into(),
            vcard: "SECRET_VCARD".into(),
        },
        status: crate::model::Delivery::None,
        delivered_at: None,
        read_at: None,
        quoted: None,
        reactions: vec![],
        edited: false,
        mentions: vec![],
        forwarded: false,
        thumbnail: Some(vec![1, 2, 3]),
    };
    let serialized = message_json(message.clone());
    assert_eq!(serialized["text"], "Visible");
    assert_eq!(serialized["sender_name"], "name");
    assert!(!serialized.to_string().contains("SECRET"));
    assert!(serialized.get("thumbnail").is_none());
    let long = Message {
        content: Content::text("a".repeat(MAX_TEXT + 1)),
        ..message
    };
    assert_eq!(
        message_json(long.clone())["text"].as_str().unwrap().len(),
        MAX_TEXT
    );
    assert_eq!(message_json(long)["text_truncated"], true);
    assert_eq!(clean("a\0\u{1b}b\n", 100), "ab\n");
}

#[test]
fn poll_projection_omits_options_and_upstream_vote_state() {
    let archive = Archive::in_memory().unwrap();
    archive.ensure_chat("fixture", "Fixture").unwrap();
    let mut poll = crate::archive::tests::message("fixture", "poll", 1, false);
    poll.content = Content::Poll {
        question: "Lunch?".into(),
        options: vec!["SECRET_OPTION".into()],
        state: crate::model::PollState {
            selectable: 1,
            counts: vec![7],
            selected: vec![0],
            voters: 7,
            can_vote: true,
            refresh_needed: true,
            ..Default::default()
        },
    };
    archive
        .insert_message(&poll, Some(b"SECRET_RAW_POLL_KEY"))
        .unwrap();
    for query in [
        Query::Recent {
            chat: "fixture".into(),
            limit: 10,
        },
        Query::Search {
            query: "Lunch".into(),
            limit: 10,
        },
    ] {
        let value = query_archive(&archive, &query).unwrap();
        let message = &value["messages"][0];
        assert_eq!(message["kind"], "poll");
        assert_eq!(message["text"], "Lunch?");
        for field in ["content", "options", "state", "counts", "selected", "raw"] {
            assert!(message.get(field).is_none());
        }
        assert!(!value.to_string().contains("SECRET"));
    }
}

#[tokio::test]
async fn requests_require_both_worker_opt_in_and_active_session() {
    let archive = Archive::in_memory().unwrap();
    for (enabled, active, expected_error) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let (reply, received) = oneshot::channel();
        Request {
            query: Query::Chats { limit: 1 },
            active: Arc::new(AtomicBool::new(active)),
            reply: Arc::new(Mutex::new(Some(reply))),
        }
        .respond(&archive, enabled);
        assert_eq!(received.await.unwrap()["isError"], expected_error);
    }
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn local_socket_requires_private_directory_and_revokes_sessions() {
    use std::{fs, os::unix::fs::PermissionsExt, time::Duration};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let root = std::env::temp_dir().join(format!(
        "zapfast-mcp-test-{}-{}",
        std::process::id(),
        jiff::Timestamp::now().as_nanosecond()
    ));
    let dirs = AppDirs::under(&root);
    dirs.ensure().unwrap();
    let (commands, mut inbox) = tokio::sync::mpsc::unbounded_channel();
    let server = Server::start(&dirs, commands.clone()).unwrap();
    let socket = dirs.state.join("mcp/socket");
    assert_eq!(
        fs::metadata(dirs.state.join("mcp"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let stream = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let mut stream = BufReader::new(stream);
    stream
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n")
        .await
        .unwrap();
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(2), stream.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"],
        json!({})
    );
    assert!(inbox.try_recv().is_err());
    drop(server);
    assert!(!socket.exists());
    line.clear();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), stream.read_line(&mut line))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    fs::set_permissions(dirs.state.join("mcp"), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(Server::start(&dirs, commands).is_err());
    fs::remove_dir_all(root).unwrap();
}
