use crate::settings::{
    ConnectionTestResult, DecisionModelSettings, ModelConnectionErrorKind, ModelConnectionTarget,
    StructuredOutputPolicy,
};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_ERROR_CHARS: usize = 2_000;
const MAX_STREAM_REPLY_CHARS: usize = 200_000;
const MAX_STREAM_EVENT_BYTES: usize = 64 * 1024;
const MAX_ORDINARY_RESPONSE_BYTES: usize = MAX_STREAM_REPLY_CHARS * 4;
const CANCEL_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(40);

pub(crate) async fn call_deepseek_api(
    system_prompt: &str,
    user_message: &str,
) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false, 0.1).await
}

pub(crate) async fn call_deepseek_api_json(
    system_prompt: &str,
    user_message: &str,
) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true, 0.5).await
}

pub(crate) async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message
    }));

    call_deepseek_api_messages(messages, force_json, temperature).await
}

pub(crate) async fn call_deepseek_api_messages(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let snapshot = crate::settings::begin_decision_request()?;
    let _settings_revision = snapshot.settings_revision;
    send_openai_compatible(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        force_json,
        temperature,
    )
    .await
    .map_err(|error| error.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamResponseError {
    Cancelled,
    Failed(String),
}

impl fmt::Display for StreamResponseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("聊天请求已取消"),
            Self::Failed(message) => formatter.write_str(message),
        }
    }
}

pub(crate) async fn call_deepseek_api_stream<F>(
    system_prompt: &str,
    user_message: &str,
    cancellation: Arc<AtomicBool>,
    on_delta: F,
) -> Result<String, StreamResponseError>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message
    }));

    let snapshot =
        crate::settings::begin_decision_request().map_err(StreamResponseError::Failed)?;
    let _settings_revision = snapshot.settings_revision;
    send_openai_compatible_stream(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        0.1,
        cancellation,
        on_delta,
    )
    .await
}

#[derive(Debug, Clone)]
struct ApiRequestError {
    kind: ModelConnectionErrorKind,
    message: String,
}

impl ApiRequestError {
    fn new(kind: ModelConnectionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for ApiRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

async fn send_openai_compatible(
    settings: &DecisionModelSettings,
    api_key: &str,
    mut messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, ApiRequestError> {
    if force_json && settings.structured_output == StructuredOutputPolicy::PromptOnly {
        messages.insert(
            0,
            serde_json::json!({
                "role": "system",
                "content": "Return only one valid JSON value. Do not wrap it in Markdown."
            }),
        );
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(settings.timeout_secs))
        .build()
        .map_err(|error| {
            ApiRequestError::new(
                ModelConnectionErrorKind::InvalidConfiguration,
                format!("构造 OpenAI Compatible HTTP 客户端失败：{error}"),
            )
        })?;

    let mut body = serde_json::json!({
        "model": settings.model,
        "messages": messages,
        "temperature": temperature,
    });
    if force_json && settings.structured_output == StructuredOutputPolicy::NativeJsonObject {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }

    let response = client
        .post(&settings.request_url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|error| classify_transport_error(error, settings.timeout_secs))?;

    parse_response(response, api_key).await
}

async fn send_openai_compatible_stream<F>(
    settings: &DecisionModelSettings,
    api_key: &str,
    messages: Vec<serde_json::Value>,
    temperature: f64,
    cancellation: Arc<AtomicBool>,
    on_delta: F,
) -> Result<String, StreamResponseError>
where
    F: FnMut(&str) -> Result<(), String>,
{
    if cancellation.load(Ordering::Acquire) {
        return Err(StreamResponseError::Cancelled);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(settings.timeout_secs))
        .build()
        .map_err(|error| {
            StreamResponseError::Failed(
                ApiRequestError::new(
                    ModelConnectionErrorKind::InvalidConfiguration,
                    format!("构造 OpenAI Compatible HTTP 客户端失败：{error}"),
                )
                .to_string(),
            )
        })?;
    let body = serde_json::json!({
        "model": settings.model,
        "messages": messages,
        "temperature": temperature,
        "stream": true,
    });
    let request = client
        .post(&settings.request_url)
        .bearer_auth(api_key)
        .json(&body)
        .send();
    tokio::pin!(request);
    let response = tokio::select! {
        result = &mut request => result
            .map_err(|error| StreamResponseError::Failed(classify_transport_error(error, settings.timeout_secs).to_string()))?,
        _ = wait_for_cancellation(&cancellation) => return Err(StreamResponseError::Cancelled),
    };

    parse_stream_response(
        response,
        api_key,
        settings.timeout_secs,
        cancellation,
        on_delta,
    )
    .await
}

async fn parse_stream_response<F>(
    mut response: reqwest::Response,
    api_key: &str,
    timeout_secs: u64,
    cancellation: Arc<AtomicBool>,
    mut on_delta: F,
) -> Result<String, StreamResponseError>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let status = response.status();
    if !status.is_success() {
        let read_body = response.text();
        tokio::pin!(read_body);
        let body = tokio::select! {
            result = &mut read_body => result.map_err(|error| {
                StreamResponseError::Failed(
                    ApiRequestError::new(
                        ModelConnectionErrorKind::Protocol,
                        format!("接口返回 HTTP {status}，且错误正文读取失败：{error}"),
                    )
                    .to_string(),
                )
            })?,
            _ = wait_for_cancellation(&cancellation) => return Err(StreamResponseError::Cancelled),
        };
        let sanitized = sanitize_api_error(&body, api_key);
        return Err(StreamResponseError::Failed(
            classify_status_error(status, &sanitized).to_string(),
        ));
    }

    let is_event_stream = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);
    let mut parser = SseParser::default();
    let mut ordinary_body = Vec::new();
    let mut reply = String::new();
    let mut reply_chars = 0usize;
    let mut saw_reasoning = false;
    let mut stream_done = false;
    let mut saw_done = false;

    while !stream_done {
        let next_chunk = tokio::select! {
            result = response.chunk() => result.map_err(|error| {
                StreamResponseError::Failed(classify_transport_error(error, timeout_secs).to_string())
            })?,
            _ = wait_for_cancellation(&cancellation) => return Err(StreamResponseError::Cancelled),
        };
        let Some(chunk) = next_chunk else {
            break;
        };

        if is_event_stream {
            let events = parser.push(&chunk).map_err(StreamResponseError::Failed)?;
            for event in events {
                match event {
                    ParsedStreamEvent::Delta { text, finished } => {
                        append_stream_delta(&mut reply, &mut reply_chars, &text, &mut on_delta)?;
                        if finished {
                            saw_done = true;
                            stream_done = true;
                            break;
                        }
                    }
                    ParsedStreamEvent::Reasoning { finished } => {
                        saw_reasoning = true;
                        if finished {
                            saw_done = true;
                            stream_done = true;
                            break;
                        }
                    }
                    ParsedStreamEvent::Done => {
                        saw_done = true;
                        stream_done = true;
                        break;
                    }
                    ParsedStreamEvent::Ignored => {}
                }
            }
        } else {
            ordinary_body.extend_from_slice(&chunk);
            if ordinary_body.len() > MAX_ORDINARY_RESPONSE_BYTES {
                return Err(protocol_stream_error("普通响应超过聊天回复长度限制"));
            }
        }
    }

    if is_event_stream {
        for event in parser.finish().map_err(StreamResponseError::Failed)? {
            match event {
                ParsedStreamEvent::Delta { text, finished } => {
                    append_stream_delta(&mut reply, &mut reply_chars, &text, &mut on_delta)?;
                    if finished {
                        saw_done = true;
                    }
                }
                ParsedStreamEvent::Reasoning { finished } => {
                    saw_reasoning = true;
                    if finished {
                        saw_done = true;
                    }
                }
                ParsedStreamEvent::Done => saw_done = true,
                ParsedStreamEvent::Ignored => {}
            }
        }
        if !saw_done {
            return Err(protocol_stream_error("流式响应在结束标记前中断"));
        }
        if reply.is_empty() {
            let message = if saw_reasoning {
                "模型完成了推理，但未返回最终答案"
            } else {
                "流式响应未包含有效文本"
            };
            return Err(protocol_stream_error(message));
        }
        return Ok(reply);
    }

    let response_data: serde_json::Value =
        serde_json::from_slice(&ordinary_body).map_err(|error| {
            protocol_stream_error(format!("解析普通 OpenAI Compatible 响应失败：{error}"))
        })?;
    let content = extract_message_content(&response_data).ok_or_else(|| {
        protocol_stream_error("OpenAI Compatible 响应缺少有效 choices[0].message.content")
    })?;
    if cancellation.load(Ordering::Acquire) {
        return Err(StreamResponseError::Cancelled);
    }
    append_stream_delta(&mut reply, &mut reply_chars, &content, &mut on_delta)?;
    Ok(reply)
}

async fn wait_for_cancellation(cancellation: &AtomicBool) {
    while !cancellation.load(Ordering::Acquire) {
        tokio::time::sleep(CANCEL_POLL_INTERVAL).await;
    }
}

fn append_stream_delta<F>(
    reply: &mut String,
    reply_chars: &mut usize,
    delta: &str,
    on_delta: &mut F,
) -> Result<(), StreamResponseError>
where
    F: FnMut(&str) -> Result<(), String>,
{
    let delta_chars = delta.chars().count();
    if delta_chars > MAX_STREAM_EVENT_BYTES
        || reply_chars.saturating_add(delta_chars) > MAX_STREAM_REPLY_CHARS
    {
        return Err(protocol_stream_error("流式回复超过长度限制"));
    }
    on_delta(delta).map_err(StreamResponseError::Failed)?;
    reply.push_str(delta);
    *reply_chars += delta_chars;
    Ok(())
}

fn protocol_stream_error(message: impl Into<String>) -> StreamResponseError {
    StreamResponseError::Failed(
        ApiRequestError::new(ModelConnectionErrorKind::Protocol, message).to_string(),
    )
}

#[derive(Debug, PartialEq, Eq)]
enum ParsedStreamEvent {
    Delta { text: String, finished: bool },
    Reasoning { finished: bool },
    Done,
    Ignored,
}

#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    fn push(&mut self, chunk: &[u8]) -> Result<Vec<ParsedStreamEvent>, String> {
        self.buffer.extend_from_slice(chunk);
        let events = self.drain_complete_events()?;
        if self.buffer.len() > MAX_STREAM_EVENT_BYTES {
            return Err("流式响应单个事件超过长度限制".to_string());
        }
        Ok(events)
    }

    fn finish(&mut self) -> Result<Vec<ParsedStreamEvent>, String> {
        let mut events = self.drain_complete_events()?;
        if !self.buffer.iter().all(u8::is_ascii_whitespace) {
            let remaining = std::mem::take(&mut self.buffer);
            events.push(parse_sse_event(&remaining)?);
        }
        Ok(events)
    }

    fn drain_complete_events(&mut self) -> Result<Vec<ParsedStreamEvent>, String> {
        let mut events = Vec::new();
        while let Some((index, delimiter_len)) = find_sse_delimiter(&self.buffer) {
            if index > MAX_STREAM_EVENT_BYTES {
                return Err("流式响应单个事件超过长度限制".to_string());
            }
            let raw_event = self.buffer[..index].to_vec();
            self.buffer.drain(..index + delimiter_len);
            events.push(parse_sse_event(&raw_event)?);
        }
        Ok(events)
    }
}

fn find_sse_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    let crlf = buffer.windows(4).position(|window| window == b"\r\n\r\n");
    let lf = buffer.windows(2).position(|window| window == b"\n\n");
    match (crlf, lf) {
        (Some(crlf), Some(lf)) if crlf <= lf => Some((crlf, 4)),
        (Some(_), Some(lf)) => Some((lf, 2)),
        (Some(crlf), None) => Some((crlf, 4)),
        (None, Some(lf)) => Some((lf, 2)),
        (None, None) => None,
    }
}

fn parse_sse_event(raw_event: &[u8]) -> Result<ParsedStreamEvent, String> {
    let event =
        std::str::from_utf8(raw_event).map_err(|_| "流式响应包含无效 UTF-8 数据".to_string())?;
    let data = event
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok(ParsedStreamEvent::Ignored);
    }
    if data.trim() == "[DONE]" {
        return Ok(ParsedStreamEvent::Done);
    }

    let value: serde_json::Value = serde_json::from_str(&data)
        .map_err(|error| format!("解析流式 OpenAI Compatible 事件失败：{error}"))?;
    let choices = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "流式 OpenAI Compatible 事件缺少 choices[0]".to_string())?;
    let Some(choice) = choices.first() else {
        return Ok(ParsedStreamEvent::Ignored);
    };
    let finished = choice
        .get("finish_reason")
        .is_some_and(|reason| !reason.is_null());
    let delta = choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(extract_content_value)
        .unwrap_or_default();
    let reasoning = choice
        .get("delta")
        .and_then(|delta| delta.get("reasoning_content"))
        .and_then(extract_content_value)
        .unwrap_or_default();
    if !delta.is_empty() {
        Ok(ParsedStreamEvent::Delta {
            text: delta,
            finished,
        })
    } else if !reasoning.is_empty() {
        Ok(ParsedStreamEvent::Reasoning { finished })
    } else if finished {
        Ok(ParsedStreamEvent::Done)
    } else {
        Ok(ParsedStreamEvent::Ignored)
    }
}

fn extract_content_value(content: &serde_json::Value) -> Option<String> {
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }
    let parts = content.as_array()?;
    Some(
        parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(serde_json::Value::as_str)
                    .or_else(|| part.get("content").and_then(serde_json::Value::as_str))
            })
            .collect::<Vec<_>>()
            .join(""),
    )
}

async fn parse_response(
    response: reqwest::Response,
    api_key: &str,
) -> Result<String, ApiRequestError> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.map_err(|error| {
            ApiRequestError::new(
                ModelConnectionErrorKind::Protocol,
                format!("接口返回 HTTP {status}，且错误正文读取失败：{error}"),
            )
        })?;
        let sanitized = sanitize_api_error(&body, api_key);
        return Err(classify_status_error(status, &sanitized));
    }

    let response_data: serde_json::Value = response.json().await.map_err(|error| {
        ApiRequestError::new(
            ModelConnectionErrorKind::Protocol,
            format!("解析 OpenAI Compatible 响应失败：{error}"),
        )
    })?;
    extract_message_content(&response_data).ok_or_else(|| {
        ApiRequestError::new(
            ModelConnectionErrorKind::Protocol,
            "OpenAI Compatible 响应缺少有效 choices[0].message.content",
        )
    })
}

fn extract_message_content(response: &serde_json::Value) -> Option<String> {
    let content = response
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?;
    if let Some(text) = content.as_str().filter(|value| !value.trim().is_empty()) {
        return Some(text.to_string());
    }
    let parts = content.as_array()?;
    let combined = parts
        .iter()
        .filter_map(|part| {
            part.get("text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| part.get("content").and_then(serde_json::Value::as_str))
        })
        .collect::<Vec<_>>()
        .join("");
    (!combined.trim().is_empty()).then_some(combined)
}

fn classify_transport_error(error: reqwest::Error, timeout_secs: u64) -> ApiRequestError {
    if error.is_timeout() {
        ApiRequestError::new(
            ModelConnectionErrorKind::Timeout,
            format!("模型接口请求超时（超过 {timeout_secs} 秒）"),
        )
    } else {
        ApiRequestError::new(
            ModelConnectionErrorKind::Network,
            format!("模型接口网络请求失败：{error}"),
        )
    }
}

fn classify_status_error(status: reqwest::StatusCode, body: &str) -> ApiRequestError {
    let body_lower = body.to_ascii_lowercase();
    let kind = match status.as_u16() {
        401 | 403 => ModelConnectionErrorKind::Authentication,
        429 if ["quota", "credit", "balance", "额度"]
            .iter()
            .any(|marker| body_lower.contains(marker)) =>
        {
            ModelConnectionErrorKind::QuotaExceeded
        }
        429 => ModelConnectionErrorKind::RateLimited,
        500..=599 => ModelConnectionErrorKind::ProviderUnavailable,
        _ => ModelConnectionErrorKind::HttpStatus,
    };
    ApiRequestError::new(kind, format!("模型接口返回 HTTP {status}：{body}"))
}

fn sanitize_api_error(value: &str, api_key: &str) -> String {
    let exact_redacted = if api_key.is_empty() {
        value.to_string()
    } else {
        value.replace(api_key, "[REDACTED]")
    };
    let bearer_redacted = redact_bearer_tokens(&exact_redacted);
    truncate_chars(&bearer_redacted, MAX_ERROR_CHARS)
}

fn redact_bearer_tokens(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut remaining = value;
    loop {
        let lower = remaining.to_ascii_lowercase();
        let Some(index) = lower.find("bearer ") else {
            result.push_str(remaining);
            break;
        };
        result.push_str(&remaining[..index]);
        result.push_str("Bearer [REDACTED]");
        let token_start = index + "bearer ".len();
        let token_end = remaining[token_start..]
            .find(|character: char| {
                character.is_whitespace()
                    || matches!(character, '"' | '\'' | ',' | '，' | ';' | '；' | '}' | ']')
            })
            .map(|offset| token_start + offset)
            .unwrap_or(remaining.len());
        remaining = &remaining[token_end..];
    }
    result
}

fn truncate_chars(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut output: String = value.chars().take(limit).collect();
    output.push_str("…[截断]");
    output
}

pub(crate) async fn test_model_connection(target: ModelConnectionTarget) -> ConnectionTestResult {
    let started = std::time::Instant::now();
    if target == ModelConnectionTarget::BuiltInGrokBuild {
        return crate::engine::test_builtin_grok_model_connection().await;
    }
    let request = crate::settings::begin_decision_request()
        .map(|snapshot| (snapshot.settings, snapshot.api_key, snapshot._activity));

    let (settings, api_key, activity) = match request {
        Ok(request) => request,
        Err(message) => {
            return ConnectionTestResult {
                success: false,
                target,
                model: String::new(),
                latency_ms: elapsed_millis(started),
                error_kind: Some(ModelConnectionErrorKind::MissingSecret),
                message,
            }
        }
    };
    let model = settings.model.clone();
    let result = send_openai_compatible(
        &settings,
        &api_key,
        vec![serde_json::json!({
            "role": "user",
            "content": "Reply with OK."
        })],
        false,
        0.0,
    )
    .await;
    drop(activity);

    match result {
        Ok(_) => ConnectionTestResult {
            success: true,
            target,
            model,
            latency_ms: elapsed_millis(started),
            error_kind: None,
            message: "连接成功".to_string(),
        },
        Err(error) => ConnectionTestResult {
            success: false,
            target,
            model,
            latency_ms: elapsed_millis(started),
            error_kind: Some(error.kind),
            message: error.message,
        },
    }
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn one_shot_server(
        status: &str,
        body: &str,
    ) -> Result<(String, tokio::task::JoinHandle<Result<String, String>>), String> {
        one_shot_server_with_content_type(status, "application/json", body).await
    }

    async fn one_shot_server_with_content_type(
        status: &str,
        content_type: &str,
        body: &str,
    ) -> Result<(String, tokio::task::JoinHandle<Result<String, String>>), String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; 16 * 1024];
            let size = socket
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            socket
                .write_all(response.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            Ok(String::from_utf8_lossy(&buffer[..size]).to_string())
        });
        Ok((format!("http://{address}/custom/chat"), handle))
    }

    async fn held_open_event_stream(
        body: &str,
    ) -> Result<(String, tokio::task::JoinHandle<Result<(), String>>), String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len() + 64
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; 16 * 1024];
            socket
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            socket
                .write_all(response.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            socket.flush().await.map_err(|error| error.to_string())?;
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            Ok(())
        });
        Ok((format!("http://{address}/custom/chat"), handle))
    }

    fn test_settings(request_url: String) -> DecisionModelSettings {
        DecisionModelSettings {
            request_url,
            timeout_secs: 5,
            ..DecisionModelSettings::default()
        }
    }

    #[tokio::test]
    async fn uses_the_configured_full_url_and_parses_text_parts() -> Result<(), String> {
        let body = r#"{"choices":[{"message":{"content":[{"text":"O"},{"text":"K"}]}}]}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let reply = send_openai_compatible(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![serde_json::json!({"role":"user","content":"hello"})],
            false,
            0.0,
        )
        .await
        .map_err(|error| error.to_string())?;
        assert_eq!(reply, "OK");
        let raw_request = request.await.map_err(|error| error.to_string())??;
        assert!(raw_request.starts_with("POST /custom/chat HTTP/1.1"));
        assert!(raw_request.contains("authorization: Bearer metheus-secret-sentinel"));
        Ok(())
    }

    #[tokio::test]
    async fn classifies_and_redacts_authentication_errors() -> Result<(), String> {
        let body = r#"{"error":"Bearer metheus-secret-sentinel is invalid"}"#;
        let (url, request) = one_shot_server("401 Unauthorized", body).await?;
        let error = send_openai_compatible(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![],
            false,
            0.0,
        )
        .await
        .err()
        .ok_or_else(|| "请求应失败".to_string())?;
        assert_eq!(error.kind, ModelConnectionErrorKind::Authentication);
        assert!(!error.message.contains("metheus-secret-sentinel"));
        assert!(error.message.contains("[REDACTED]"));
        request
            .await
            .map_err(|join_error| join_error.to_string())??;
        Ok(())
    }

    #[test]
    fn prompt_only_policy_does_not_require_native_json_support() {
        let settings = DecisionModelSettings {
            structured_output: StructuredOutputPolicy::PromptOnly,
            ..DecisionModelSettings::default()
        };
        assert_eq!(
            settings.structured_output,
            StructuredOutputPolicy::PromptOnly
        );
    }

    #[test]
    fn bearer_redaction_is_case_insensitive_and_unicode_safe() {
        let value = redact_bearer_tokens("错误 bearer secret-token，稍后重试");
        assert_eq!(value, "错误 Bearer [REDACTED]，稍后重试");
    }

    #[test]
    fn sse_parser_preserves_utf8_and_event_boundaries_across_chunks() -> Result<(), String> {
        let mut parser = SseParser::default();
        let payload = "data: {\"choices\":[{\"delta\":{\"content\":\"你好\"}}]}\n\n";
        let bytes = payload.as_bytes();
        let split = payload
            .find('好')
            .ok_or_else(|| "缺少测试字符".to_string())?
            + 1;

        assert!(parser.push(&bytes[..split])?.is_empty());
        assert_eq!(
            parser.push(&bytes[split..])?,
            vec![ParsedStreamEvent::Delta {
                text: "你好".to_string(),
                finished: false,
            }]
        );
        Ok(())
    }

    #[test]
    fn sse_parser_ignores_reasoning_and_metadata_without_treating_them_as_text(
    ) -> Result<(), String> {
        let mut parser = SseParser::default();
        let payload = concat!(
            ": keep-alive\n\n",
            "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"completion_tokens\":1}}\n\n",
        );

        assert_eq!(
            parser.push(payload.as_bytes())?,
            vec![
                ParsedStreamEvent::Ignored,
                ParsedStreamEvent::Ignored,
                ParsedStreamEvent::Reasoning { finished: false },
                ParsedStreamEvent::Ignored,
            ]
        );
        Ok(())
    }

    #[test]
    fn sse_parser_recognizes_done_and_finish_reason() -> Result<(), String> {
        let mut parser = SseParser::default();
        assert_eq!(
            parser.push(b"data: [DONE]\r\n\r\n")?,
            vec![ParsedStreamEvent::Done]
        );
        assert_eq!(
            parser.push(b"data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n")?,
            vec![ParsedStreamEvent::Done]
        );
        Ok(())
    }

    #[test]
    fn sse_parser_rejects_invalid_protocol() {
        let mut parser = SseParser::default();
        let error = parser
            .push(b"data: not-json\n\n")
            .expect_err("无效事件必须失败");
        assert!(error.contains("解析流式"));
    }

    #[test]
    fn sse_parser_rejects_a_complete_oversized_event() {
        let mut parser = SseParser::default();
        let oversized = format!("data: {}\n\n", "x".repeat(MAX_STREAM_EVENT_BYTES + 1));
        let error = parser
            .push(oversized.as_bytes())
            .expect_err("超长完整事件必须失败");
        assert!(error.contains("单个事件超过长度限制"));
    }

    #[tokio::test]
    async fn stream_reader_falls_back_to_ordinary_json() -> Result<(), String> {
        let body = r#"{"choices":[{"message":{"content":"whole reply"}}]}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let cancellation = Arc::new(AtomicBool::new(false));
        let mut deltas = Vec::new();
        let reply = send_openai_compatible_stream(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![serde_json::json!({"role":"user","content":"hello"})],
            0.0,
            cancellation,
            |delta| {
                deltas.push(delta.to_string());
                Ok(())
            },
        )
        .await
        .map_err(|error| error.to_string())?;

        assert_eq!(reply, "whole reply");
        assert_eq!(deltas, vec!["whole reply"]);
        let raw_request = request.await.map_err(|error| error.to_string())??;
        assert!(raw_request.contains("\"stream\":true"));
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_accepts_long_reasoning_before_visible_content() -> Result<(), String> {
        let mut body = String::new();
        for index in 0..160 {
            body.push_str(&format!(
                "data: {{\"choices\":[{{\"delta\":{{\"reasoning_content\":\"thought-{index}\"}},\"finish_reason\":null}}]}}\n\n"
            ));
        }
        body.push_str(
            "data: {\"choices\":[{\"delta\":{\"content\":\"final answer\"},\"finish_reason\":null}]}\n\n",
        );
        body.push_str("data: [DONE]\n\n");
        let (url, request) =
            one_shot_server_with_content_type("200 OK", "text/event-stream", &body).await?;
        let mut deltas = Vec::new();
        let reply = send_openai_compatible_stream(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |delta| {
                deltas.push(delta.to_string());
                Ok(())
            },
        )
        .await
        .map_err(|error| error.to_string())?;

        assert_eq!(reply, "final answer");
        assert_eq!(deltas, vec!["final answer"]);
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_accepts_content_and_finish_reason_in_the_same_event(
    ) -> Result<(), String> {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"complete\"},\"finish_reason\":\"stop\"}]}\n\n";
        let (url, request) =
            one_shot_server_with_content_type("200 OK", "text/event-stream", body).await?;
        let reply = send_openai_compatible_stream(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .await
        .map_err(|error| error.to_string())?;

        assert_eq!(reply, "complete");
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_reports_reasoning_only_responses_clearly() -> Result<(), String> {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        );
        let (url, request) =
            one_shot_server_with_content_type("200 OK", "text/event-stream", body).await?;
        let result = send_openai_compatible_stream(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .await;

        assert!(matches!(
            result,
            Err(StreamResponseError::Failed(message))
                if message.contains("模型完成了推理，但未返回最终答案")
        ));
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_honors_preflight_cancellation() {
        let settings = test_settings("http://127.0.0.1:9/never".to_string());
        let cancellation = Arc::new(AtomicBool::new(true));
        let result =
            send_openai_compatible_stream(&settings, "secret", vec![], 0.0, cancellation, |_| {
                Ok(())
            })
            .await;
        assert_eq!(result, Err(StreamResponseError::Cancelled));
    }

    #[tokio::test]
    async fn stream_reader_honors_cancellation_after_a_partial_delta() -> Result<(), String> {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        let (url, request) = held_open_event_stream(body).await?;
        let cancellation = Arc::new(AtomicBool::new(false));
        let cancel_after_delta = Arc::clone(&cancellation);
        let mut deltas = Vec::new();
        let result = send_openai_compatible_stream(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            cancellation,
            |delta| {
                deltas.push(delta.to_string());
                cancel_after_delta.store(true, Ordering::Release);
                Ok(())
            },
        )
        .await;

        assert_eq!(result, Err(StreamResponseError::Cancelled));
        assert_eq!(deltas, vec!["partial"]);
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_rejects_a_connection_closed_before_done() -> Result<(), String> {
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n";
        let (url, request) =
            one_shot_server_with_content_type("200 OK", "text/event-stream", body).await?;
        let mut deltas = Vec::new();
        let result = send_openai_compatible_stream(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |delta| {
                deltas.push(delta.to_string());
                Ok(())
            },
        )
        .await;

        assert!(matches!(
            result,
            Err(StreamResponseError::Failed(message)) if message.contains("结束标记前中断")
        ));
        assert_eq!(deltas, vec!["partial"]);
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }
}
