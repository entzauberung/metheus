//! Audited in-process embedding facade for Metheus.
//!
//! The facade drives the normal Grok Build `SessionActor` and routes every
//! filesystem operation through an ACP client that enforces the frozen
//! project and write policy. No shell configuration, session persistence,
//! terminal implementation, MCP server, hook, skill, plugin, or subagent is
//! exposed to an embedded session.

use crate::agent::MvpAgent;
use crate::session::{PromptCompletionKind, SessionCommand};
use agent_client_protocol as acp;
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use xai_grok_tools::implementations::metheus_embedded::EMBEDDED_PATH_NOT_FOUND;
use xai_grok_tools::implementations::metheus_embedded::EmbeddedFilePolicy;

pub const FORK_REVISION: &str = "metheus.3";
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_EVENT_TEXT_CHARS: usize = 2_000;
const TRUNCATION_SUFFIX: &str = "...[truncated]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedApiBackend {
    ChatCompletions,
    Responses,
    Messages,
}

#[derive(Clone)]
pub struct EmbeddedConfig {
    pub api_backend: EmbeddedApiBackend,
    pub api_base_url: String,
    pub model: String,
    pub api_key: String,
    pub timeout: Duration,
    pub max_turns: usize,
    pub max_transport_retries: u32,
    pub max_doom_loop_retries: u32,
}

impl fmt::Debug for EmbeddedConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddedConfig")
            .field("api_backend", &self.api_backend)
            .field("api_base_url", &self.api_base_url)
            .field("model", &self.model)
            .field("api_key_configured", &!self.api_key.is_empty())
            .field("timeout", &self.timeout)
            .field("max_turns", &self.max_turns)
            .field("max_transport_retries", &self.max_transport_retries)
            .field("max_doom_loop_retries", &self.max_doom_loop_retries)
            .finish()
    }
}

#[derive(Clone)]
pub struct EmbeddedRequest {
    pub project_root: PathBuf,
    pub authorized_write_paths: Vec<PathBuf>,
    pub prompt: String,
    pub execution_id: String,
    pub cancellation: Arc<AtomicBool>,
    pub event_sink: Option<EmbeddedEventSink>,
}

impl fmt::Debug for EmbeddedRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmbeddedRequest")
            .field("project_root", &self.project_root)
            .field("authorized_write_paths", &self.authorized_write_paths)
            .field("prompt_chars", &self.prompt.chars().count())
            .field("execution_id", &self.execution_id)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbeddedEvent {
    ModelText(String),
    ToolStarted(String),
    ToolCompleted(String),
    ToolFailed(String),
    RetryScheduled {
        attempt: u32,
        max_retries: u32,
        reason: String,
    },
    RetryExhausted {
        attempts: u32,
        reason: String,
        is_rate_limited: bool,
    },
    RetryFailed {
        error_type: String,
        message: String,
    },
}

#[derive(Clone)]
pub struct EmbeddedEventSink {
    sender: std::sync::mpsc::Sender<EmbeddedEventDispatch>,
}

enum EmbeddedEventDispatch {
    Event(EmbeddedEvent),
    Flush(std::sync::mpsc::SyncSender<()>),
}

impl EmbeddedEventSink {
    pub fn new(callback: impl Fn(EmbeddedEvent) + Send + Sync + 'static) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let _ = std::thread::Builder::new()
            .name("metheus-grok-embedded-events".to_string())
            .spawn(move || {
                while let Ok(dispatch) = receiver.recv() {
                    match dispatch {
                        EmbeddedEventDispatch::Event(event) => callback(event),
                        EmbeddedEventDispatch::Flush(completed) => {
                            let _ = completed.send(());
                        }
                    }
                }
            });
        Self { sender }
    }

    fn emit(&self, event: EmbeddedEvent) {
        // Never run the host callback on SessionActor. The unbounded queue keeps notification
        // delivery non-blocking; a stopped worker simply closes it.
        let _ = self.sender.send(EmbeddedEventDispatch::Event(event));
    }

    /// Wait until every event previously emitted by SessionActor has reached the host callback.
    /// This is called only after the actor has returned, before the adapter publishes its final
    /// result, so it cannot add backpressure to SessionActor.
    pub fn flush(&self) {
        let (completed, receiver) = std::sync::mpsc::sync_channel(0);
        if self
            .sender
            .send(EmbeddedEventDispatch::Flush(completed))
            .is_ok()
        {
            let _ = receiver.recv();
        }
    }
}

impl fmt::Debug for EmbeddedEventSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EmbeddedEventSink(..)")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddedErrorKind {
    InvalidConfiguration,
    Authentication,
    QuotaExceeded,
    RateLimited,
    Network,
    ProviderUnavailable,
    Timeout,
    Cancelled,
    ToolRejected,
    Protocol,
    MaxTurns,
    Runtime,
}

#[derive(Debug, Clone)]
pub struct EmbeddedError {
    pub kind: EmbeddedErrorKind,
    message: String,
}

impl EmbeddedError {
    pub fn message(&self) -> &str {
        &self.message
    }

    fn new(kind: EmbeddedErrorKind, message: impl Into<String>, api_key: &str) -> Self {
        let mut message = message.into();
        if !api_key.is_empty() {
            message = message.replace(api_key, "[REDACTED]");
        }
        if message.chars().count() > 2_000 {
            message = format!(
                "{}...[truncated]",
                message.chars().take(2_000).collect::<String>()
            );
        }
        Self { kind, message }
    }

    fn from_acp(error: acp::Error, api_key: &str) -> Self {
        let rendered = error.data.as_ref().map_or_else(
            || error.message.clone(),
            |data| format!("{}: {data}", error.message),
        );
        classify_error(rendered, api_key)
    }
}

impl std::fmt::Display for EmbeddedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EmbeddedError {}

#[derive(Debug, Clone)]
pub struct EmbeddedResult {
    pub output: String,
    pub turns: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub files_written: Vec<String>,
    pub stop_reason: String,
}

pub async fn execute(
    config: EmbeddedConfig,
    request: EmbeddedRequest,
) -> Result<EmbeddedResult, EmbeddedError> {
    validate(&config, &request)?;
    let api_key = config.api_key.clone();
    let (result_tx, result_rx) = oneshot::channel();
    std::thread::Builder::new()
        .name("metheus-grok-embedded".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    EmbeddedError::new(
                        EmbeddedErrorKind::Runtime,
                        format!("Cannot initialize embedded Grok Build runtime: {error}"),
                        &config.api_key,
                    )
                })
                .and_then(|runtime| {
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&runtime, execute_local(config, request))
                });
            let _ = result_tx.send(result);
        })
        .map_err(|error| {
            EmbeddedError::new(
                EmbeddedErrorKind::Runtime,
                format!("Cannot start embedded Grok Build thread: {error}"),
                &api_key,
            )
        })?;
    result_rx.await.map_err(|_| {
        EmbeddedError::new(
            EmbeddedErrorKind::Runtime,
            "Embedded Grok Build thread exited without a result",
            &api_key,
        )
    })?
}

async fn execute_local(
    config: EmbeddedConfig,
    request: EmbeddedRequest,
) -> Result<EmbeddedResult, EmbeddedError> {
    let policy = EmbeddedFilePolicy::new(&request.project_root, &request.authorized_write_paths)
        .map_err(|message| {
            EmbeddedError::new(EmbeddedErrorKind::ToolRejected, message, &config.api_key)
        })?;
    let client = Arc::new(RestrictedClient::new(
        policy.clone(),
        request.event_sink.clone(),
        config.api_key.clone(),
    ));
    let (gateway, receiver) = xai_acp_lib::acp_gateway::<acp::AgentSide, _>(client.clone());
    let gateway_task = tokio::task::spawn_local(receiver.run());
    let sampling_config = xai_grok_sampler::SamplerConfig {
        api_key: Some(config.api_key.clone()),
        base_url: config.api_base_url.trim_end_matches('/').to_string(),
        model: config.model.clone(),
        max_completion_tokens: Some(16_384),
        temperature: Some(0.0),
        api_backend: match config.api_backend {
            EmbeddedApiBackend::ChatCompletions => xai_grok_sampler::ApiBackend::ChatCompletions,
            EmbeddedApiBackend::Responses => xai_grok_sampler::ApiBackend::Responses,
            EmbeddedApiBackend::Messages => xai_grok_sampler::ApiBackend::Messages,
        },
        auth_scheme: if config.api_backend == EmbeddedApiBackend::Messages {
            xai_grok_sampler::AuthScheme::XApiKey
        } else {
            xai_grok_sampler::AuthScheme::Bearer
        },
        context_window: 128_000,
        max_retries: Some(config.max_transport_retries),
        stream_tool_calls: true,
        idle_timeout_secs: Some(config.timeout.as_secs()),
        client_identifier: Some("metheus-embedded".to_string()),
        client_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        doom_loop_recovery: (config.max_doom_loop_retries > 0).then(|| {
            xai_grok_sampling_types::DoomLoopRecoveryPolicy {
                max_threshold:
                    xai_grok_sampling_types::DoomLoopRecoveryPolicy::DEFAULT_MAX_THRESHOLD,
                max_retries: xai_grok_sampling_types::DoomLoopRecoveryPolicy::clamp_max_retries(
                    config.max_doom_loop_retries,
                ),
            }
        }),
        ..Default::default()
    };
    let max_turns = u32::try_from(config.max_turns).map_err(|_| {
        EmbeddedError::new(
            EmbeddedErrorKind::InvalidConfiguration,
            "Grok Build maximum turns exceeds the supported range",
            &config.api_key,
        )
    })?;
    let auth_root = tempfile::tempdir().map_err(|error| {
        EmbeddedError::new(
            EmbeddedErrorKind::Runtime,
            format!("Cannot create embedded authentication directory: {error}"),
            &config.api_key,
        )
    })?;
    let agent =
        MvpAgent::new_metheus_embedded(gateway, sampling_config, max_turns, auth_root.path())
            .map_err(|message| {
                EmbeddedError::new(EmbeddedErrorKind::Runtime, message, &config.api_key)
            })?;
    let session_id = acp::SessionId::new(format!("metheus-{}", request.execution_id));
    agent
        .spawn_metheus_embedded(
            policy.root().to_path_buf(),
            session_id.clone(),
            policy.clone(),
        )
        .await
        .map_err(|error| EmbeddedError::from_acp(error, &config.api_key))?;
    let handle = agent.embedded_session_handle(&session_id).ok_or_else(|| {
        EmbeddedError::new(
            EmbeddedErrorKind::Runtime,
            "Embedded Grok Build session was not registered",
            &config.api_key,
        )
    })?;

    let authorized = request
        .authorized_write_paths
        .iter()
        .map(|path| format!("- {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let prompt = format!(
        "Work only inside this project: {}\nAuthorized write paths:\n{}\n\n{}",
        policy.root().display(),
        if authorized.is_empty() {
            "- none"
        } else {
            &authorized
        },
        request.prompt,
    );
    let (respond_to, response_rx) = oneshot::channel();
    handle
        .cmd_tx
        .send(SessionCommand::Prompt {
            prompt_id: format!("metheus-{}", request.execution_id),
            prompt_blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(prompt))],
            prompt_mode: crate::session::plan_mode::PromptMode::Agent,
            artifact_upload_ctx: None,
            client_identifier: Some("metheus-embedded".to_string()),
            screen_mode: Some("headless".to_string()),
            verbatim: true,
            traceparent: None,
            json_schema: None,
            send_now: false,
            admission: None,
            respond_to,
            persist_ack: None,
            parsed_prompt_tx: None,
        })
        .map_err(|_| {
            EmbeddedError::new(
                EmbeddedErrorKind::Runtime,
                "Embedded Grok Build session rejected the prompt",
                &config.api_key,
            )
        })?;

    let cancel_tx = handle.cmd_tx.clone();
    let cancellation = request.cancellation.clone();
    let cancel_task = tokio::task::spawn_local(async move {
        while !cancellation.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = cancel_tx.send(SessionCommand::Cancel {
            cancel_subagents: true,
            kill_background_tasks: true,
            rewind_if_pristine: false,
            trigger: Some("metheus".to_string()),
        });
    });
    let policy_cancel_tx = handle.cmd_tx.clone();
    let policy_client = client.clone();
    let policy_task = tokio::task::spawn_local(async move {
        policy_client.policy_notify.notified().await;
        let _ = policy_cancel_tx.send(SessionCommand::Cancel {
            cancel_subagents: true,
            kill_background_tasks: true,
            rewind_if_pristine: false,
            trigger: Some("metheus-policy".to_string()),
        });
    });
    let response = match tokio::time::timeout(config.timeout, response_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(acp::Error::internal_error().data("embedded prompt response dropped")),
        Err(_) => {
            let _ = handle.cmd_tx.send(SessionCommand::Cancel {
                cancel_subagents: true,
                kill_background_tasks: true,
                rewind_if_pristine: false,
                trigger: Some("timeout".to_string()),
            });
            cancel_task.abort();
            policy_task.abort();
            agent.shutdown_metheus_embedded(&session_id).await;
            gateway_task.abort();
            return Err(EmbeddedError::new(
                EmbeddedErrorKind::Timeout,
                format!(
                    "Grok Build execution timed out after {} seconds",
                    config.timeout.as_secs()
                ),
                &config.api_key,
            ));
        }
    };
    cancel_task.abort();
    policy_task.abort();
    agent.shutdown_metheus_embedded(&session_id).await;
    gateway_task.abort();
    if let Some(message) = client
        .policy_violation
        .lock()
        .ok()
        .and_then(|value| value.clone())
    {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::ToolRejected,
            message,
            &config.api_key,
        ));
    }
    let response = response.map_err(|error| EmbeddedError::from_acp(error, &config.api_key))?;
    let (turns, prompt_tokens, completion_tokens) =
        response
            .usage
            .as_ref()
            .map_or((1, response.total_tokens, 0), |usage| {
                (
                    usage.num_turns.max(1) as u32,
                    usage.totals.input_tokens,
                    usage.totals.output_tokens,
                )
            });
    match response.completion_kind {
        PromptCompletionKind::Cancelled { .. } => {
            return Err(EmbeddedError::new(
                EmbeddedErrorKind::Cancelled,
                "Grok Build execution was cancelled",
                &config.api_key,
            ));
        }
        PromptCompletionKind::MaxTurnsReached { limit } => {
            return Err(EmbeddedError::new(
                EmbeddedErrorKind::MaxTurns,
                format!("Grok Build reached the configured maximum of {limit} turns"),
                &config.api_key,
            ));
        }
        PromptCompletionKind::RemovedFromQueue | PromptCompletionKind::Rewound => {
            return Err(EmbeddedError::new(
                EmbeddedErrorKind::Protocol,
                "Grok Build did not execute the embedded prompt",
                &config.api_key,
            ));
        }
        PromptCompletionKind::Completed => {}
    }
    Ok(EmbeddedResult {
        output: client
            .output
            .lock()
            .map(|value| value.text.clone())
            .unwrap_or_default(),
        turns,
        prompt_tokens,
        completion_tokens,
        files_written: client
            .written
            .lock()
            .map(|paths| paths.iter().cloned().collect())
            .unwrap_or_default(),
        stop_reason: format!("{:?}", response.stop_reason),
    })
}

fn validate(config: &EmbeddedConfig, request: &EmbeddedRequest) -> Result<(), EmbeddedError> {
    if config.api_key.trim().is_empty() {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::Authentication,
            "Grok Build API key is not configured",
            "",
        ));
    }
    if config.api_key.chars().any(char::is_control) {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::Authentication,
            "Grok Build API key contains invalid control characters",
            &config.api_key,
        ));
    }
    if config.model.trim().is_empty() || config.api_base_url.trim().is_empty() {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::InvalidConfiguration,
            "Grok Build API base URL and model are required",
            &config.api_key,
        ));
    }
    let endpoint = url::Url::parse(config.api_base_url.trim()).map_err(|error| {
        EmbeddedError::new(
            EmbeddedErrorKind::InvalidConfiguration,
            format!("Grok Build API base URL is invalid: {error}"),
            &config.api_key,
        )
    })?;
    if !matches!(endpoint.scheme(), "http" | "https")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::InvalidConfiguration,
            "Grok Build API base URL must be an HTTP(S) URL without credentials, query, or fragment",
            &config.api_key,
        ));
    }
    if config.timeout.is_zero() || config.max_turns == 0 || request.prompt.trim().is_empty() {
        return Err(EmbeddedError::new(
            EmbeddedErrorKind::InvalidConfiguration,
            "Grok Build timeout, maximum turns, and prompt must be non-empty",
            &config.api_key,
        ));
    }
    Ok(())
}

struct RestrictedClient {
    policy: EmbeddedFilePolicy,
    output: Mutex<OutputState>,
    written: Mutex<BTreeSet<String>>,
    tool_names: Mutex<HashMap<String, String>>,
    policy_violation: Mutex<Option<String>>,
    policy_notify: tokio::sync::Notify,
    event_sink: Option<EmbeddedEventSink>,
    api_key: String,
    last_retry_attempt: Mutex<u32>,
}

#[derive(Default)]
struct OutputState {
    text: String,
    attempt_start: Option<usize>,
}

impl OutputState {
    fn push_attempt_text(&mut self, text: &str) {
        self.attempt_start.get_or_insert(self.text.len());
        self.text.push_str(text);
    }

    fn accept_attempt(&mut self) {
        self.attempt_start = None;
    }

    fn discard_attempt(&mut self) {
        if let Some(start) = self.attempt_start {
            self.text.truncate(start);
        }
    }
}

impl RestrictedClient {
    fn new(
        policy: EmbeddedFilePolicy,
        event_sink: Option<EmbeddedEventSink>,
        api_key: String,
    ) -> Self {
        Self {
            policy,
            output: Mutex::new(OutputState::default()),
            written: Mutex::new(BTreeSet::new()),
            tool_names: Mutex::new(HashMap::new()),
            policy_violation: Mutex::new(None),
            policy_notify: tokio::sync::Notify::new(),
            event_sink,
            api_key,
            last_retry_attempt: Mutex::new(0),
        }
    }

    fn emit(&self, event: EmbeddedEvent) {
        if let Some(sink) = &self.event_sink {
            sink.emit(event);
        }
    }

    fn sanitize_event_text(&self, value: impl Into<String>) -> String {
        let value = value.into();
        let redacted = if self.api_key.is_empty() {
            value
        } else {
            value.replace(&self.api_key, "[REDACTED]")
        };
        if redacted.chars().count() <= MAX_EVENT_TEXT_CHARS {
            redacted
        } else {
            format!(
                "{}{}",
                redacted
                    .chars()
                    .take(MAX_EVENT_TEXT_CHARS - TRUNCATION_SUFFIX.chars().count())
                    .collect::<String>(),
                TRUNCATION_SUFFIX,
            )
        }
    }

    fn reject_policy(&self, message: impl Into<String>) -> acp::Error {
        let message = message.into();
        if let Ok(mut violation) = self.policy_violation.lock()
            && violation.is_none()
        {
            *violation = Some(message.clone());
        }
        self.policy_notify.notify_one();
        rpc_permission_denied(message)
    }
}

#[async_trait::async_trait(?Send)]
impl acp::Client for RestrictedClient {
    async fn request_permission(
        &self,
        _args: acp::RequestPermissionRequest,
    ) -> acp::Result<acp::RequestPermissionResponse> {
        Err(self.reject_policy("interactive permission requests are disabled"))
    }

    async fn session_notification(&self, args: acp::SessionNotification) -> acp::Result<()> {
        match args.update {
            acp::SessionUpdate::AgentMessageChunk(chunk) => {
                if let Ok(mut attempt) = self.last_retry_attempt.lock() {
                    *attempt = 0;
                }
                if let acp::ContentBlock::Text(text) = chunk.content {
                    if let Ok(mut output) = self.output.lock() {
                        output.push_attempt_text(&text.text);
                    }
                    self.emit(EmbeddedEvent::ModelText(text.text));
                }
            }
            acp::SessionUpdate::ToolCall(call) => {
                if let Ok(mut attempt) = self.last_retry_attempt.lock() {
                    *attempt = 0;
                }
                if let Ok(mut output) = self.output.lock() {
                    output.accept_attempt();
                }
                let id = call.tool_call_id.to_string();
                if let Ok(mut names) = self.tool_names.lock() {
                    names.insert(id, call.title.clone());
                }
                self.emit(EmbeddedEvent::ToolStarted(call.title));
            }
            acp::SessionUpdate::ToolCallUpdate(update)
                if matches!(
                    update.fields.status,
                    Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)
                ) =>
            {
                let id = update.tool_call_id.to_string();
                let remembered_name = self
                    .tool_names
                    .lock()
                    .ok()
                    .and_then(|mut names| names.remove(&id));
                let name = update
                    .fields
                    .title
                    .or(remembered_name)
                    .unwrap_or_else(|| "tool".to_string());
                match update.fields.status {
                    Some(acp::ToolCallStatus::Completed) => {
                        self.emit(EmbeddedEvent::ToolCompleted(name));
                    }
                    Some(acp::ToolCallStatus::Failed) => {
                        self.emit(EmbeddedEvent::ToolFailed(name));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        Ok(())
    }

    async fn ext_notification(&self, args: acp::ExtNotification) -> acp::Result<()> {
        if args.method.as_ref() != "x.ai/session_notification" {
            return Ok(());
        }
        let Ok(notification) = serde_json::from_str::<
            crate::extensions::notification::SessionNotification,
        >(args.params.get()) else {
            return Ok(());
        };
        let crate::extensions::notification::SessionUpdate::RetryState(state) = notification.update
        else {
            return Ok(());
        };
        use crate::extensions::notification::RetryState;
        let event = match state {
            RetryState::Retrying {
                attempt,
                max_retries,
                reason,
            } => {
                if let Ok(mut output) = self.output.lock() {
                    output.discard_attempt();
                }
                if let Ok(mut last_attempt) = self.last_retry_attempt.lock() {
                    *last_attempt = attempt;
                }
                EmbeddedEvent::RetryScheduled {
                    attempt,
                    max_retries,
                    reason: self.sanitize_event_text(reason),
                }
            }
            RetryState::Exhausted {
                attempts,
                reason,
                is_rate_limited,
            } => {
                let attempts = if let Ok(mut last_attempt) = self.last_retry_attempt.lock() {
                    let normalized = if attempts == 0 {
                        last_attempt.saturating_add(1).max(1)
                    } else {
                        attempts
                    };
                    *last_attempt = 0;
                    normalized
                } else {
                    attempts.max(1)
                };
                EmbeddedEvent::RetryExhausted {
                    attempts,
                    reason: self.sanitize_event_text(reason),
                    is_rate_limited,
                }
            }
            RetryState::Failed {
                error_type,
                message,
            } => {
                if let Ok(mut attempt) = self.last_retry_attempt.lock() {
                    *attempt = 0;
                }
                EmbeddedEvent::RetryFailed {
                    error_type: self.sanitize_event_text(error_type),
                    message: self.sanitize_event_text(message),
                }
            }
        };
        self.emit(event);
        Ok(())
    }

    async fn read_text_file(
        &self,
        args: acp::ReadTextFileRequest,
    ) -> acp::Result<acp::ReadTextFileResponse> {
        match self
            .policy
            .read_text_file(&args.path, args.line, args.limit)
        {
            Ok(content) => Ok(acp::ReadTextFileResponse::new(content)),
            Err(message) if message == EMBEDDED_PATH_NOT_FOUND => Err(acp::Error::new(
                acp::ErrorCode::ResourceNotFound.into(),
                message,
            )),
            Err(message) => Err(self.reject_policy(message)),
        }
    }

    async fn write_text_file(
        &self,
        args: acp::WriteTextFileRequest,
    ) -> acp::Result<acp::WriteTextFileResponse> {
        let target = self
            .policy
            .write_text_file(&args.path, &args.content)
            .map_err(|message| self.reject_policy(message))?;
        if let Ok(relative) = target.strip_prefix(self.policy.root())
            && let Ok(mut written) = self.written.lock()
        {
            written.insert(relative.to_string_lossy().to_string());
        }
        Ok(acp::WriteTextFileResponse::new())
    }

    async fn create_terminal(
        &self,
        _args: acp::CreateTerminalRequest,
    ) -> acp::Result<acp::CreateTerminalResponse> {
        Err(self.reject_policy("terminal execution is disabled"))
    }
}

fn rpc_permission_denied(message: impl Into<String>) -> acp::Error {
    let mut error = acp::Error::invalid_params();
    error.message = format!("permission denied: {}", message.into());
    error
}

fn classify_error(message: String, api_key: &str) -> EmbeddedError {
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("cancel") {
        EmbeddedErrorKind::Cancelled
    } else if lower.contains("401") || lower.contains("403") || lower.contains("auth") {
        EmbeddedErrorKind::Authentication
    } else if lower.contains("quota") || lower.contains("credit") || lower.contains("billing") {
        EmbeddedErrorKind::QuotaExceeded
    } else if lower.contains("429") || lower.contains("rate limit") {
        EmbeddedErrorKind::RateLimited
    } else if lower.contains("timeout") || lower.contains("timed out") {
        EmbeddedErrorKind::Timeout
    } else if lower.contains("connection") || lower.contains("dns") || lower.contains("network") {
        EmbeddedErrorKind::Network
    } else if lower.contains("500") || lower.contains("502") || lower.contains("503") {
        EmbeddedErrorKind::ProviderUnavailable
    } else if lower.contains("permission denied") {
        EmbeddedErrorKind::ToolRejected
    } else {
        EmbeddedErrorKind::Protocol
    };
    EmbeddedError::new(kind, message, api_key)
}
