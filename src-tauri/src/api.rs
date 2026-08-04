use crate::cost_ledger::{ModelCallContext, ModelCallMetadata, ModelCallResponse, ProviderUsage};
use crate::settings::{
    ConnectionTestResult, DecisionModelSettings, ModelConnectionErrorKind, ModelConnectionTarget,
    StructuredOutputPolicy,
};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const MAX_ERROR_CHARS: usize = 2_000;
const MAX_RESPONSE_DIAGNOSTIC_BYTES: usize = 500;
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

/// 审查链路使用的结构化入口。其他业务继续接收字符串错误，避免改变既有行为。
pub(crate) async fn call_deepseek_api_json_typed(
    system_prompt: &str,
    user_message: &str,
) -> Result<String, ApiRequestError> {
    call_deepseek_api_inner_typed(system_prompt, user_message, true, 0.5).await
}

pub(crate) async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    call_deepseek_api_inner_typed(system_prompt, user_message, force_json, temperature)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn call_deepseek_api_inner_typed(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, ApiRequestError> {
    call_deepseek_api_inner_typed_with_context(
        system_prompt,
        user_message,
        force_json,
        temperature,
        ModelCallContext::default(),
    )
    .await
    .map(|response| response.content)
}

pub(crate) async fn call_deepseek_api_inner_with_context(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
    context: ModelCallContext,
) -> Result<ModelCallResponse, String> {
    call_deepseek_api_inner_typed_with_context(
        system_prompt,
        user_message,
        force_json,
        temperature,
        context,
    )
    .await
    .map_err(|error| error.to_string())
}

pub(crate) async fn call_deepseek_api_json_with_context(
    system_prompt: &str,
    user_message: &str,
    context: ModelCallContext,
) -> Result<ModelCallResponse, String> {
    call_deepseek_api_inner_with_context(system_prompt, user_message, true, 0.5, context).await
}

pub(crate) async fn call_deepseek_api_json_typed_with_context(
    system_prompt: &str,
    user_message: &str,
    context: ModelCallContext,
) -> Result<ModelCallResponse, ApiRequestError> {
    call_deepseek_api_inner_typed_with_context(system_prompt, user_message, true, 0.5, context)
        .await
}

pub(crate) async fn call_deepseek_api_inner_typed_with_context(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
    context: ModelCallContext,
) -> Result<ModelCallResponse, ApiRequestError> {
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

    let snapshot = crate::settings::begin_decision_request().map_err(|message| {
        ApiRequestError::new(ModelConnectionErrorKind::MissingSecret, message)
    })?;
    let _settings_revision = snapshot.settings_revision;
    let call_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now().to_rfc3339();
    let started = std::time::Instant::now();
    let result = send_openai_compatible_with_usage(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        force_json,
        temperature,
    )
    .await;
    let ended_at = chrono::Utc::now().to_rfc3339();
    let elapsed_ms = elapsed_millis(started);
    match result {
        Ok(response) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: if response.model.is_empty() {
                    snapshot.settings.model.clone()
                } else {
                    response.model
                },
                provider_response_id: response.provider_response_id,
                started_at,
                ended_at,
                elapsed_ms,
                usage: response.usage,
                failure_kind: String::new(),
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            Ok(ModelCallResponse {
                content: response.content,
                metadata,
            })
        }
        Err(mut error) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: snapshot.settings.model.clone(),
                provider_response_id: String::new(),
                started_at,
                ended_at,
                elapsed_ms,
                usage: None,
                failure_kind: format!("{:?}", error.kind),
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            error.metadata = Some(metadata);
            Err(error)
        }
    }
}

pub(crate) async fn call_deepseek_api_messages(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    call_deepseek_api_messages_with_context(
        messages,
        force_json,
        temperature,
        ModelCallContext::default(),
    )
    .await
    .map(|response| response.content)
}

pub(crate) async fn call_deepseek_api_messages_with_context(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
    context: ModelCallContext,
) -> Result<ModelCallResponse, String> {
    call_deepseek_api_messages_typed_with_context(messages, force_json, temperature, context)
        .await
        .map_err(|error| error.to_string())
}

pub(crate) async fn call_deepseek_api_messages_typed_with_context(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
    context: ModelCallContext,
) -> Result<ModelCallResponse, ApiRequestError> {
    let snapshot = crate::settings::begin_decision_request().map_err(|message| {
        ApiRequestError::new(ModelConnectionErrorKind::MissingSecret, message)
    })?;
    let _settings_revision = snapshot.settings_revision;
    let started_at = chrono::Utc::now().to_rfc3339();
    let started = std::time::Instant::now();
    let call_id = uuid::Uuid::new_v4().to_string();
    let result = send_openai_compatible_with_usage(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        force_json,
        temperature,
    )
    .await;
    let ended_at = chrono::Utc::now().to_rfc3339();
    let elapsed_ms = elapsed_millis(started);
    match result {
        Ok(response) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: if response.model.is_empty() {
                    snapshot.settings.model.clone()
                } else {
                    response.model
                },
                provider_response_id: response.provider_response_id,
                started_at,
                ended_at,
                elapsed_ms,
                usage: response.usage,
                failure_kind: String::new(),
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            Ok(ModelCallResponse {
                content: response.content,
                metadata,
            })
        }
        Err(mut error) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: snapshot.settings.model.clone(),
                provider_response_id: String::new(),
                started_at,
                ended_at,
                elapsed_ms,
                usage: None,
                failure_kind: format!("{:?}", error.kind),
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            error.metadata = Some(metadata);
            Err(error)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StreamResponseError {
    Cancelled,
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct StreamModelCallError {
    error: StreamResponseError,
    metadata: ModelCallMetadata,
}

impl StreamModelCallError {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.error == StreamResponseError::Cancelled
    }

    pub(crate) fn metadata(&self) -> &ModelCallMetadata {
        &self.metadata
    }
}

impl fmt::Display for StreamModelCallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
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
    call_deepseek_api_stream_with_context(
        system_prompt,
        user_message,
        cancellation,
        on_delta,
        ModelCallContext::default(),
    )
    .await
    .map(|response| response.content)
    .map_err(|error| error.error)
}

pub(crate) async fn call_deepseek_api_stream_with_context<F>(
    system_prompt: &str,
    user_message: &str,
    cancellation: Arc<AtomicBool>,
    on_delta: F,
    context: ModelCallContext,
) -> Result<ModelCallResponse, StreamModelCallError>
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

    let call_id = uuid::Uuid::new_v4().to_string();
    let started_at = chrono::Utc::now().to_rfc3339();
    let started = std::time::Instant::now();
    let snapshot = match crate::settings::begin_decision_request() {
        Ok(snapshot) => snapshot,
        Err(message) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                started_at,
                ended_at: chrono::Utc::now().to_rfc3339(),
                elapsed_ms: elapsed_millis(started),
                failure_kind: "MissingSecret".to_string(),
                ..Default::default()
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            return Err(StreamModelCallError {
                error: StreamResponseError::Failed(message),
                metadata,
            });
        }
    };
    let _settings_revision = snapshot.settings_revision;
    let result = send_openai_compatible_stream_with_usage(
        &snapshot.settings,
        &snapshot.api_key,
        messages,
        0.1,
        cancellation,
        on_delta,
    )
    .await;
    let ended_at = chrono::Utc::now().to_rfc3339();
    let elapsed_ms = elapsed_millis(started);
    match result {
        Ok(response) => {
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: if response.model.is_empty() {
                    snapshot.settings.model.clone()
                } else {
                    response.model
                },
                provider_response_id: response.provider_response_id,
                started_at,
                ended_at,
                elapsed_ms,
                usage: response.usage,
                failure_kind: String::new(),
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            Ok(ModelCallResponse {
                content: response.content,
                metadata,
            })
        }
        Err(error) => {
            let failure_kind = if error == StreamResponseError::Cancelled {
                "Cancelled"
            } else {
                "StreamFailed"
            };
            let metadata = ModelCallMetadata {
                call_id,
                context,
                model: snapshot.settings.model.clone(),
                started_at,
                ended_at,
                elapsed_ms,
                failure_kind: failure_kind.to_string(),
                ..Default::default()
            };
            crate::cost_ledger::record_metadata_best_effort(&metadata);
            Err(StreamModelCallError { error, metadata })
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ApiRequestError {
    kind: ModelConnectionErrorKind,
    message: String,
    metadata: Option<ModelCallMetadata>,
}

impl ApiRequestError {
    fn new(kind: ModelConnectionErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            metadata: None,
        }
    }

    pub(crate) fn review_failure_kind(&self) -> crate::project::ReviewFailureKind {
        match self.kind {
            ModelConnectionErrorKind::Network => crate::project::ReviewFailureKind::Network,
            ModelConnectionErrorKind::Timeout => crate::project::ReviewFailureKind::Timeout,
            ModelConnectionErrorKind::RateLimited => crate::project::ReviewFailureKind::RateLimited,
            ModelConnectionErrorKind::Authentication
            | ModelConnectionErrorKind::MissingSecret
            | ModelConnectionErrorKind::InvalidConfiguration => {
                crate::project::ReviewFailureKind::Authentication
            }
            ModelConnectionErrorKind::QuotaExceeded => {
                crate::project::ReviewFailureKind::QuotaExceeded
            }
            ModelConnectionErrorKind::ProviderUnavailable
            | ModelConnectionErrorKind::Protocol
            | ModelConnectionErrorKind::HttpStatus => {
                crate::project::ReviewFailureKind::ServiceUnavailable
            }
        }
    }

    pub(crate) fn diagnostic_summary(&self) -> &str {
        &self.message
    }

    pub(crate) fn call_metadata(&self) -> Option<&ModelCallMetadata> {
        self.metadata.as_ref()
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
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, ApiRequestError> {
    send_openai_compatible_with_usage(settings, api_key, messages, force_json, temperature)
        .await
        .map(|response| response.content)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct ProviderResponse {
    content: String,
    usage: Option<ProviderUsage>,
    provider_response_id: String,
    model: String,
}

async fn send_openai_compatible_with_usage(
    settings: &DecisionModelSettings,
    api_key: &str,
    mut messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<ProviderResponse, ApiRequestError> {
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

    parse_response_with_usage(response, api_key, settings.timeout_secs).await
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
    send_openai_compatible_stream_with_usage(
        settings,
        api_key,
        messages,
        temperature,
        cancellation,
        on_delta,
    )
    .await
    .map(|response| response.content)
}

async fn send_openai_compatible_stream_with_usage<F>(
    settings: &DecisionModelSettings,
    api_key: &str,
    messages: Vec<serde_json::Value>,
    temperature: f64,
    cancellation: Arc<AtomicBool>,
    on_delta: F,
) -> Result<ProviderResponse, StreamResponseError>
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

    parse_stream_response_with_usage(
        response,
        api_key,
        settings.timeout_secs,
        cancellation,
        on_delta,
    )
    .await
}

async fn parse_stream_response_with_usage<F>(
    mut response: reqwest::Response,
    api_key: &str,
    timeout_secs: u64,
    cancellation: Arc<AtomicBool>,
    mut on_delta: F,
) -> Result<ProviderResponse, StreamResponseError>
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
                        }
                    }
                    ParsedStreamEvent::Reasoning { finished } => {
                        saw_reasoning = true;
                        if finished {
                            saw_done = true;
                        }
                    }
                    ParsedStreamEvent::Finished => saw_done = true,
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
                ParsedStreamEvent::Finished | ParsedStreamEvent::Done => saw_done = true,
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
        let mut response = parser.provider_response;
        response.content = reply;
        return Ok(response);
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
    Ok(ProviderResponse {
        content: reply,
        usage: extract_provider_usage(&response_data),
        provider_response_id: response_data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: response_data
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
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
    Finished,
    Done,
    Ignored,
}

#[derive(Default)]
struct SseParser {
    buffer: Vec<u8>,
    provider_response: ProviderResponse,
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
            let (event, metadata) = parse_sse_event(&remaining)?;
            self.merge_provider_metadata(metadata);
            events.push(event);
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
            let (event, metadata) = parse_sse_event(&raw_event)?;
            self.merge_provider_metadata(metadata);
            events.push(event);
        }
        Ok(events)
    }

    fn merge_provider_metadata(&mut self, metadata: ProviderResponse) {
        if metadata.usage.is_some() {
            self.provider_response.usage = metadata.usage;
        }
        if !metadata.provider_response_id.is_empty() {
            self.provider_response.provider_response_id = metadata.provider_response_id;
        }
        if !metadata.model.is_empty() {
            self.provider_response.model = metadata.model;
        }
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

fn parse_sse_event(raw_event: &[u8]) -> Result<(ParsedStreamEvent, ProviderResponse), String> {
    let event =
        std::str::from_utf8(raw_event).map_err(|_| "流式响应包含无效 UTF-8 数据".to_string())?;
    let data = event
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
        .collect::<Vec<_>>()
        .join("\n");
    if data.is_empty() {
        return Ok((ParsedStreamEvent::Ignored, ProviderResponse::default()));
    }
    if data.trim() == "[DONE]" {
        return Ok((ParsedStreamEvent::Done, ProviderResponse::default()));
    }

    let value: serde_json::Value = serde_json::from_str(&data)
        .map_err(|error| format!("解析流式 OpenAI Compatible 事件失败：{error}"))?;
    let metadata = ProviderResponse {
        usage: extract_provider_usage(&value),
        provider_response_id: value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: value
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        ..Default::default()
    };
    let choices = match value.get("choices").and_then(serde_json::Value::as_array) {
        Some(choices) => choices,
        None if metadata.usage.is_some() => {
            return Ok((ParsedStreamEvent::Ignored, metadata));
        }
        None => return Err("流式 OpenAI Compatible 事件缺少 choices[0]".to_string()),
    };
    let Some(choice) = choices.first() else {
        return Ok((ParsedStreamEvent::Ignored, metadata));
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
        Ok((
            ParsedStreamEvent::Delta {
                text: delta,
                finished,
            },
            metadata,
        ))
    } else if !reasoning.is_empty() {
        Ok((ParsedStreamEvent::Reasoning { finished }, metadata))
    } else if finished {
        Ok((ParsedStreamEvent::Finished, metadata))
    } else {
        Ok((ParsedStreamEvent::Ignored, metadata))
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

async fn parse_response_with_usage(
    response: reqwest::Response,
    api_key: &str,
    timeout_secs: u64,
) -> Result<ProviderResponse, ApiRequestError> {
    parse_response_with_usage_timeout(
        response,
        api_key,
        std::time::Duration::from_secs(timeout_secs),
    )
    .await
}

async fn parse_response_with_usage_timeout(
    response: reqwest::Response,
    api_key: &str,
    read_timeout: std::time::Duration,
) -> Result<ProviderResponse, ApiRequestError> {
    let status = response.status();
    let response_bytes = match tokio::time::timeout(read_timeout, response.bytes()).await {
        Err(_) => {
            return Err(ApiRequestError::new(
                ModelConnectionErrorKind::Timeout,
                format!(
                    "读取 OpenAI Compatible 响应超时（超过 {} 秒）",
                    read_timeout.as_secs_f64()
                ),
            ));
        }
        Ok(Err(error)) if error.is_timeout() => {
            return Err(ApiRequestError::new(
                ModelConnectionErrorKind::Timeout,
                "读取 OpenAI Compatible 响应超时",
            ));
        }
        Ok(Err(error)) => {
            let reason = if error.is_body() || error.is_decode() {
                "响应正文未完整到达"
            } else {
                "连接在读取响应正文时中断"
            };
            return Err(ApiRequestError::new(
                ModelConnectionErrorKind::Network,
                format!("OpenAI Compatible 响应网络读取失败：{reason}"),
            ));
        }
        Ok(Ok(bytes)) => bytes,
    };

    if !status.is_success() {
        let body = String::from_utf8_lossy(&response_bytes);
        let sanitized = sanitize_api_error(&body, api_key);
        return Err(classify_status_error(status, &sanitized));
    }

    if response_bytes.is_empty() {
        return Err(ApiRequestError::new(
            ModelConnectionErrorKind::Network,
            "OpenAI Compatible 响应正文为空，可能在正文到达前断开连接",
        ));
    }

    let response_text = String::from_utf8_lossy(&response_bytes);
    let first_non_whitespace = response_text.trim_start().chars().next();
    if !matches!(first_non_whitespace, Some('{') | Some('[')) {
        let diagnostic = response_body_diagnostic(&response_bytes, api_key);
        return Err(ApiRequestError::new(
            ModelConnectionErrorKind::ProviderUnavailable,
            format!(
                "OpenAI Compatible 服务返回了非 JSON 正文；响应前缀（已脱敏，最多 500 字节）：{diagnostic}"
            ),
        ));
    }

    let response_data: serde_json::Value = match serde_json::from_slice(&response_bytes) {
        Ok(value) => value,
        Err(error) if error.classify() == serde_json::error::Category::Eof => {
            let diagnostic = response_body_diagnostic(&response_bytes, api_key);
            return Err(ApiRequestError::new(
                ModelConnectionErrorKind::Network,
                format!(
                    "OpenAI Compatible 响应在 JSON 完成前中断；响应前缀（已脱敏，最多 500 字节）：{diagnostic}"
                ),
            ));
        }
        Err(initial_error) => {
            // 与上层 Schema 修复链共用同一确定性清洗入口；仅完整但形态错误的
            // JSON 进入该路径，EOF/网络截断不会消耗修复机会。
            let cleaned = crate::json_utils::sanitize_json_response(&response_text);
            match serde_json::from_str(&cleaned) {
                Ok(value) => value,
                Err(cleaned_error) => {
                    let diagnostic = response_body_diagnostic(&response_bytes, api_key);
                    return Err(ApiRequestError::new(
                        ModelConnectionErrorKind::Protocol,
                        format!(
                            "解析 OpenAI Compatible JSON 响应失败：{initial_error}；确定性清洗后仍失败：{cleaned_error}；响应前缀（已脱敏，最多 500 字节）：{diagnostic}"
                        ),
                    ));
                }
            }
        }
    };
    let content = extract_message_content(&response_data).ok_or_else(|| {
        ApiRequestError::new(
            ModelConnectionErrorKind::Protocol,
            "OpenAI Compatible 响应缺少有效 choices[0].message.content",
        )
    })?;
    Ok(ProviderResponse {
        content,
        usage: extract_provider_usage(&response_data),
        provider_response_id: response_data
            .get("id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
        model: response_data
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn extract_provider_usage(response: &serde_json::Value) -> Option<ProviderUsage> {
    let usage = response.get("usage")?.as_object()?;
    let input_tokens = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(serde_json::Value::as_u64);
    let output_tokens = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(serde_json::Value::as_u64);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            input_tokens
                .zip(output_tokens)
                .map(|(input, output)| input + output)
        });
    let cached_input_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|details| details.get("cached_tokens"))
        .or_else(|| {
            usage
                .get("input_tokens_details")
                .and_then(|details| details.get("cached_tokens"))
        })
        .and_then(serde_json::Value::as_u64);
    if input_tokens.is_none()
        && output_tokens.is_none()
        && total_tokens.is_none()
        && cached_input_tokens.is_none()
    {
        return None;
    }
    Some(ProviderUsage {
        input_tokens,
        output_tokens,
        total_tokens,
        cached_input_tokens,
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

fn response_body_diagnostic(bytes: &[u8], api_key: &str) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let sanitized = sanitize_api_error(&raw, api_key);
    truncate_utf8_bytes(&sanitized, MAX_RESPONSE_DIAGNOSTIC_BYTES)
}

fn truncate_utf8_bytes(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[截断]", &value[..end])
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

    async fn delayed_response_body(
        body: &'static str,
        delay: std::time::Duration,
    ) -> Result<(String, tokio::task::JoinHandle<Result<(), String>>), String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|error| error.to_string())?;
        let address = listener.local_addr().map_err(|error| error.to_string())?;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let handle = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.map_err(|error| error.to_string())?;
            let mut buffer = vec![0u8; 16 * 1024];
            socket
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            socket
                .write_all(headers.as_bytes())
                .await
                .map_err(|error| error.to_string())?;
            socket.flush().await.map_err(|error| error.to_string())?;
            tokio::time::sleep(delay).await;
            let _ = socket.write_all(body.as_bytes()).await;
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
    async fn parses_provider_usage_without_exposing_request_content() -> Result<(), String> {
        let body = r#"{"id":"req-1","model":"provider-model","choices":[{"message":{"content":"OK"}}],"usage":{"prompt_tokens":12,"completion_tokens":5,"total_tokens":17,"prompt_tokens_details":{"cached_tokens":4}}}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let response = send_openai_compatible_with_usage(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![serde_json::json!({"role":"user","content":"sensitive prompt"})],
            false,
            0.0,
        )
        .await
        .map_err(|error| error.to_string())?;
        assert_eq!(response.content, "OK");
        assert_eq!(response.provider_response_id, "req-1");
        assert_eq!(response.model, "provider-model");
        assert_eq!(
            response.usage.as_ref().and_then(|usage| usage.input_tokens),
            Some(12)
        );
        assert_eq!(
            response
                .usage
                .as_ref()
                .and_then(|usage| usage.output_tokens),
            Some(5)
        );
        assert_eq!(
            response.usage.as_ref().and_then(|usage| usage.total_tokens),
            Some(17)
        );
        assert_eq!(
            response
                .usage
                .as_ref()
                .and_then(|usage| usage.cached_input_tokens),
            Some(4)
        );
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[test]
    fn missing_or_invalid_usage_remains_unknown() {
        assert!(extract_provider_usage(&serde_json::json!({})).is_none());
        assert!(extract_provider_usage(&serde_json::json!({
            "usage": { "prompt_tokens": "unknown", "completion_tokens": null }
        }))
        .is_none());
        let derived = extract_provider_usage(&serde_json::json!({
            "usage": { "input_tokens": 8, "output_tokens": 3 }
        }))
        .unwrap();
        assert_eq!(derived.total_tokens, Some(11));
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

    #[tokio::test]
    async fn runtime_fix_classifies_empty_success_body_as_network_failure() -> Result<(), String> {
        let (url, request) = one_shot_server("200 OK", "").await?;
        let error = send_openai_compatible(&test_settings(url), "secret", vec![], false, 0.0)
            .await
            .expect_err("空响应必须失败");

        assert_eq!(error.kind, ModelConnectionErrorKind::Network);
        assert!(error.message.contains("响应正文为空"));
        assert!(!error.message.contains("error decoding response body"));
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_fix_classifies_truncated_json_as_network_with_prefix() -> Result<(), String> {
        let body = r#"{"choices":[{"message":{"content":"partial"}}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let error = send_openai_compatible(&test_settings(url), "secret", vec![], false, 0.0)
            .await
            .expect_err("截断 JSON 必须失败");

        assert_eq!(error.kind, ModelConnectionErrorKind::Network);
        assert!(error.message.contains("JSON 完成前中断"));
        assert!(error.message.contains("partial"));
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_fix_classifies_html_as_unavailable_and_redacts_prefix() -> Result<(), String> {
        let body = "<html>gateway echoed Bearer metheus-secret-sentinel</html>";
        let (url, request) = one_shot_server_with_content_type("200 OK", "text/html", body).await?;
        let error = send_openai_compatible(
            &test_settings(url),
            "metheus-secret-sentinel",
            vec![],
            false,
            0.0,
        )
        .await
        .expect_err("HTML 正文必须失败");

        assert_eq!(error.kind, ModelConnectionErrorKind::ProviderUnavailable);
        assert!(error.message.contains("非 JSON 正文"));
        assert!(error.message.contains("[REDACTED]"));
        assert!(!error.message.contains("metheus-secret-sentinel"));
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_fix_classifies_malformed_json_as_protocol_with_prefix() -> Result<(), String> {
        let body = r#"{"choices":!}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let error = send_openai_compatible(&test_settings(url), "secret", vec![], false, 0.0)
            .await
            .expect_err("形态错误 JSON 必须失败");

        assert_eq!(error.kind, ModelConnectionErrorKind::Protocol);
        assert!(error.message.contains("确定性清洗后仍失败"));
        assert!(error.message.contains(body));
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn runtime_fix_classifies_body_timeout_without_json_repair() -> Result<(), String> {
        let (url, request) = delayed_response_body(
            r#"{"choices":[{"message":{"content":"late"}}]}"#,
            std::time::Duration::from_millis(100),
        )
        .await?;
        let response = reqwest::Client::new()
            .post(url)
            .send()
            .await
            .map_err(|error| error.to_string())?;
        let error = parse_response_with_usage_timeout(
            response,
            "secret",
            std::time::Duration::from_millis(10),
        )
        .await
        .expect_err("正文读取超时必须失败");

        assert_eq!(error.kind, ModelConnectionErrorKind::Timeout);
        assert!(error.message.contains("读取 OpenAI Compatible 响应超时"));
        assert!(!error.message.contains("JSON"));
        request.await.map_err(|error| error.to_string())??;
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
            "data: {\"usage\":{\"completion_tokens\":2}}\n\n",
        );

        assert_eq!(
            parser.push(payload.as_bytes())?,
            vec![
                ParsedStreamEvent::Ignored,
                ParsedStreamEvent::Ignored,
                ParsedStreamEvent::Reasoning { finished: false },
                ParsedStreamEvent::Ignored,
                ParsedStreamEvent::Ignored,
            ]
        );
        assert_eq!(
            parser
                .provider_response
                .usage
                .as_ref()
                .and_then(|usage| usage.output_tokens),
            Some(2)
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
            vec![ParsedStreamEvent::Finished]
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
        assert!(!raw_request.contains("stream_options"));
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_collects_metadata_from_ordinary_json() -> Result<(), String> {
        let body = r#"{"id":"ordinary-1","model":"ordinary-model","choices":[{"message":{"content":"whole reply"}}],"usage":{"prompt_tokens":7,"completion_tokens":2,"total_tokens":9}}"#;
        let (url, request) = one_shot_server("200 OK", body).await?;
        let response = send_openai_compatible_stream_with_usage(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .await
        .map_err(|error| error.to_string())?;

        assert_eq!(response.content, "whole reply");
        assert_eq!(response.provider_response_id, "ordinary-1");
        assert_eq!(response.model, "ordinary-model");
        assert_eq!(
            response.usage.as_ref().and_then(|usage| usage.total_tokens),
            Some(9)
        );
        request.await.map_err(|error| error.to_string())??;
        Ok(())
    }

    #[tokio::test]
    async fn stream_reader_collects_trailing_usage_event() -> Result<(), String> {
        let body = concat!(
            "data: {\"id\":\"stream-1\",\"model\":\"stream-model\",\"choices\":[{\"delta\":{\"content\":\"done\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":3,\"total_tokens\":14}}\n\n",
            "data: [DONE]\n\n",
        );
        let (url, request) =
            one_shot_server_with_content_type("200 OK", "text/event-stream", body).await?;
        let response = send_openai_compatible_stream_with_usage(
            &test_settings(url),
            "secret",
            vec![],
            0.0,
            Arc::new(AtomicBool::new(false)),
            |_| Ok(()),
        )
        .await
        .map_err(|error| error.to_string())?;

        assert_eq!(response.content, "done");
        assert_eq!(response.provider_response_id, "stream-1");
        assert_eq!(response.model, "stream-model");
        assert_eq!(
            response.usage.as_ref().and_then(|usage| usage.total_tokens),
            Some(14)
        );
        request.await.map_err(|error| error.to_string())??;
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
