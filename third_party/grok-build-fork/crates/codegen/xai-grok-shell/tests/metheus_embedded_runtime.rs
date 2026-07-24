#![cfg(feature = "metheus-embedded")]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use xai_grok_shell::metheus_embedded::{
    EmbeddedApiBackend, EmbeddedConfig, EmbeddedErrorKind, EmbeddedRequest, execute,
};

fn read_request(stream: &mut TcpStream) -> std::io::Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4_096];
    let mut expected = None;
    loop {
        let size = stream.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..size]);
        if expected.is_none()
            && let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n")
        {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            });
            expected = Some(header_end + 4 + content_length.unwrap_or(0));
        }
        if expected.is_some_and(|length| bytes.len() >= length) {
            break;
        }
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn sse_response(events: &[serde_json::Value]) -> String {
    let mut body = events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect::<String>();
    body.push_str("data: [DONE]\n\n");
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
}

fn tool_turn() -> String {
    sse_response(&[
        serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "delta": {
                    "role": "assistant",
                    "tool_calls": [
                        {
                            "index": 0,
                            "id": "call-read",
                            "type": "function",
                            "function": {
                                "name": "read_file",
                                "arguments": "{\"target_file\":\"seed.txt\"}"
                            }
                        },
                        {
                            "index": 1,
                            "id": "call-list",
                            "type": "function",
                            "function": {
                                "name": "list_dir",
                                "arguments": "{\"target_directory\":\".\"}"
                            }
                        },
                        {
                            "index": 2,
                            "id": "call-grep",
                            "type": "function",
                            "function": {
                                "name": "grep",
                                "arguments": "{\"pattern\":\"embedded seed\",\"path\":\".\"}"
                            }
                        },
                        {
                            "index": 3,
                            "id": "call-write",
                            "type": "function",
                            "function": {
                                "name": "search_replace",
                                "arguments": "{\"file_path\":\"allowed.txt\",\"old_string\":\"\",\"new_string\":\"written by session actor\",\"replace_all\":false}"
                            }
                        }
                    ]
                },
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl-tool",
            "object": "chat.completion.chunk",
            "created": 1,
            "model": "test-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14}
        }),
    ])
}

fn text_turn() -> String {
    sse_response(&[
        serde_json::json!({
            "id": "chatcmpl-done",
            "object": "chat.completion.chunk",
            "created": 2,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant", "content": "done"},
                "finish_reason": null
            }]
        }),
        serde_json::json!({
            "id": "chatcmpl-done",
            "object": "chat.completion.chunk",
            "created": 2,
            "model": "test-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 20, "completion_tokens": 2, "total_tokens": 22}
        }),
    ])
}

fn test_config(address: SocketAddr) -> EmbeddedConfig {
    EmbeddedConfig {
        api_backend: EmbeddedApiBackend::ChatCompletions,
        api_base_url: format!("http://{address}/v1"),
        model: "test-model".to_string(),
        api_key: "local-test-secret".to_string(),
        timeout: Duration::from_secs(5),
        max_turns: 3,
    }
}

fn test_request(root: &Path, execution_id: &str, cancellation: Arc<AtomicBool>) -> EmbeddedRequest {
    EmbeddedRequest {
        project_root: root.to_path_buf(),
        authorized_write_paths: Vec::new(),
        prompt: "Inspect the project, then finish.".to_string(),
        execution_id: execution_id.to_string(),
        cancellation,
        event_sink: None,
    }
}

#[test]
fn embedded_config_debug_redacts_api_key() {
    let config = EmbeddedConfig {
        api_key: "metheus-secret-sentinel".to_string(),
        ..test_config("127.0.0.1:1".parse().expect("socket address"))
    };
    let debug = format!("{config:?}");
    assert!(!debug.contains("metheus-secret-sentinel"));
    assert!(debug.contains("api_key_configured: true"));
}

#[tokio::test]
async fn drives_upstream_session_actor_through_exact_four_tool_boundary()
-> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let (request_tx, request_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        for response in [tool_turn(), text_turn()] {
            let (mut stream, _) = listener.accept()?;
            request_tx
                .send(read_request(&mut stream)?)
                .map_err(std::io::Error::other)?;
            stream.write_all(response.as_bytes())?;
        }
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    std::fs::write(directory.path().join("seed.txt"), "embedded seed\n")?;
    let result = execute(
        EmbeddedConfig {
            timeout: Duration::from_secs(10),
            ..test_config(address)
        },
        EmbeddedRequest {
            project_root: directory.path().to_path_buf(),
            authorized_write_paths: vec![PathBuf::from("allowed.txt")],
            prompt: "Create the authorized file, then finish.".to_string(),
            execution_id: "session-actor-integration".to_string(),
            cancellation: Arc::new(AtomicBool::new(false)),
            event_sink: None,
        },
    )
    .await?;
    server.join().map_err(|_| "server thread panicked")??;

    let first_request = request_rx.recv_timeout(Duration::from_secs(1))?;
    let authorization_headers = first_request
        .lines()
        .filter(|line| line.to_ascii_lowercase().starts_with("authorization:"))
        .collect::<Vec<_>>();
    assert_eq!(authorization_headers.len(), 1);
    assert_eq!(
        authorization_headers[0].to_ascii_lowercase(),
        "authorization: bearer local-test-secret"
    );
    let lower_request = first_request.to_ascii_lowercase();
    for forbidden in [
        "x-api-key:",
        "x-xai-token-auth:",
        "x-authenticateresponse:",
        "x-grok-deployment-id:",
    ] {
        assert!(!lower_request.contains(forbidden));
    }
    let request_body = first_request
        .split_once("\r\n\r\n")
        .map(|(_, body)| body)
        .ok_or("model request did not contain an HTTP body")?;
    let request_json: serde_json::Value = serde_json::from_str(request_body)?;
    let mut tool_names = request_json["tools"]
        .as_array()
        .ok_or("model request did not contain tools")?
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();
    tool_names.sort_unstable();
    assert_eq!(
        tool_names,
        ["grep", "list_dir", "read_file", "search_replace"]
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("allowed.txt"))?,
        "written by session actor"
    );
    assert!(result.output.contains("done"));
    assert_eq!(result.files_written, vec!["allowed.txt"]);
    Ok(())
}

#[tokio::test]
async fn cancellation_interrupts_active_upstream_turn() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let cancellation = Arc::new(AtomicBool::new(false));
    let server_cancellation = cancellation.clone();
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let _ = read_request(&mut stream)?;
        server_cancellation.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    let result = execute(
        test_config(address),
        test_request(directory.path(), "cancel-test", cancellation),
    )
    .await;
    server.join().map_err(|_| "server thread panicked")??;
    assert_eq!(
        result
            .expect_err("cancellation must interrupt the turn")
            .kind,
        EmbeddedErrorKind::Cancelled
    );
    Ok(())
}

#[tokio::test]
async fn timeout_and_shutdown_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let _ = read_request(&mut stream)?;
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    let mut config = test_config(address);
    config.timeout = Duration::from_millis(75);
    let started = std::time::Instant::now();
    let result = execute(
        config,
        test_request(
            directory.path(),
            "timeout-test",
            Arc::new(AtomicBool::new(false)),
        ),
    )
    .await;
    let elapsed = started.elapsed();
    server.join().map_err(|_| "server thread panicked")??;
    assert_eq!(
        result.expect_err("stalled sampling must time out").kind,
        EmbeddedErrorKind::Timeout
    );
    assert!(
        elapsed < Duration::from_secs(3),
        "shutdown took {elapsed:?}"
    );
    Ok(())
}

#[tokio::test]
async fn upstream_session_actor_enforces_max_turns() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let _ = read_request(&mut stream)?;
        stream.write_all(tool_turn().as_bytes())?;
        Ok(())
    });
    let directory = tempfile::tempdir()?;
    std::fs::write(directory.path().join("seed.txt"), "seed\n")?;
    let mut config = test_config(address);
    config.max_turns = 1;
    let result = execute(
        config,
        test_request(
            directory.path(),
            "max-turns-test",
            Arc::new(AtomicBool::new(false)),
        ),
    )
    .await;
    server.join().map_err(|_| "server thread panicked")??;
    assert_eq!(
        result.expect_err("turn limit must stop the loop").kind,
        EmbeddedErrorKind::MaxTurns
    );
    Ok(())
}
