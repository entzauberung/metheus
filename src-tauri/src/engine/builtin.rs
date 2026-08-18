use super::contract::{
    EngineAuthState, EngineAuthVerificationMethod, EngineAuthenticationResult, EngineError,
    EngineHealth, EngineHealthStatus, EngineLocalAuthState, EngineOnlineAuthState,
    EngineRuntimeSelfTestResult, EngineRuntimeSelfTestState, ExecutionRequest,
};
use crate::pipeline::{append_runtime_log, PipelineState, PipelineStatus};
use crate::project::{EngineFailureKind, ExecutionProvider, ExecutionResult, ExecutionRuntime};
use crate::runtime_resource::ResourceDecision;
use crate::settings::{
    AppSettings, BuiltInGrokBuildSettings, ConnectionTestResult, GrokBuildApiBackend,
    ModelConnectionErrorKind, ModelConnectionTarget,
};
use crate::task_contract::{ExecutionGuard, ExecutionStopReason};
use metheus_grok_engine::{
    GrokBuildExecutionConfig, GrokBuildExecutionRequest, GrokBuildRuntimeErrorKind,
    GrokBuildRuntimeEvent, RuntimeEventSink, TokenUsage,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

#[derive(Clone, PartialEq, Eq)]
struct SelfTestCacheIdentity {
    settings_revision: u64,
    api_backend: String,
    endpoint_fingerprint: String,
    model: String,
    timeout_secs: u64,
    max_turns: u32,
    source_revision: String,
    api_key_digest: [u8; 32],
}

impl SelfTestCacheIdentity {
    fn new(settings: &AppSettings, api_key: &str) -> Self {
        Self {
            settings_revision: settings.revision,
            api_backend: settings
                .built_in_grok_build
                .api_backend
                .as_str()
                .to_string(),
            endpoint_fingerprint: crate::settings::endpoint_fingerprint(
                &settings.built_in_grok_build.api_base_url,
            ),
            model: settings.built_in_grok_build.model.clone(),
            timeout_secs: settings.built_in_grok_build.timeout_secs,
            max_turns: settings.built_in_grok_build.max_turns,
            source_revision: metheus_grok_engine::source_revision().to_string(),
            api_key_digest: Sha256::digest(api_key.as_bytes()).into(),
        }
    }
}

#[derive(Clone)]
struct CachedSelfTest {
    identity: SelfTestCacheIdentity,
    result: EngineRuntimeSelfTestResult,
}

static SELF_TEST_CACHE: OnceLock<Mutex<Option<CachedSelfTest>>> = OnceLock::new();

pub(super) const fn is_compiled() -> bool {
    true
}

pub(super) fn source_revision() -> Option<String> {
    Some(metheus_grok_engine::source_revision().to_string())
}

fn cached_self_test(settings: &AppSettings, api_key: &str) -> Option<EngineRuntimeSelfTestResult> {
    let identity = SelfTestCacheIdentity::new(settings, api_key);
    SELF_TEST_CACHE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()?
        .as_ref()
        .filter(|record| record.identity == identity)
        .map(|record| record.result.clone())
}

fn cache_self_test(settings: &AppSettings, api_key: &str, result: EngineRuntimeSelfTestResult) {
    if let Ok(mut cache) = SELF_TEST_CACHE.get_or_init(|| Mutex::new(None)).lock() {
        *cache = Some(CachedSelfTest {
            identity: SelfTestCacheIdentity::new(settings, api_key),
            result,
        });
    }
}

fn adapter_config(
    settings: &BuiltInGrokBuildSettings,
    api_key: &str,
    max_turns: u32,
    max_transport_retries: u32,
    max_doom_loop_retries: u32,
) -> GrokBuildExecutionConfig {
    GrokBuildExecutionConfig {
        api_backend: match settings.api_backend {
            GrokBuildApiBackend::ChatCompletions => {
                metheus_grok_engine::GrokBuildApiBackend::ChatCompletions
            }
            GrokBuildApiBackend::Responses => metheus_grok_engine::GrokBuildApiBackend::Responses,
            GrokBuildApiBackend::Messages => metheus_grok_engine::GrokBuildApiBackend::Messages,
        },
        api_base_url: settings.api_base_url.clone(),
        model: settings.model.clone(),
        api_key: api_key.to_string(),
        timeout_secs: settings.timeout_secs,
        max_turns,
        max_transport_retries,
        max_doom_loop_retries,
    }
}

pub(super) fn health(settings: &AppSettings, api_key: Option<&str>) -> EngineHealth {
    let source_revision = metheus_grok_engine::source_revision().to_string();
    let secret_configured = api_key.is_some();
    let self_test = api_key.and_then(|api_key| cached_self_test(settings, api_key));
    let self_test_state = self_test
        .as_ref()
        .map(|result| result.state.clone())
        .unwrap_or_default();
    let (status, auth_state, message) = if !secret_configured {
        (
            EngineHealthStatus::Unauthenticated,
            EngineAuthState::Unauthenticated,
            "预装 Grok Build API Key 未配置".to_string(),
        )
    } else {
        match self_test.as_ref() {
            Some(result) if result.success => (
                EngineHealthStatus::Available,
                EngineAuthState::Authenticated,
                format!(
                    "预装 Grok Build 运行时可用 · 源码 {}",
                    &source_revision[..8]
                ),
            ),
            Some(result) => (
                EngineHealthStatus::VerificationFailed,
                EngineAuthState::Unknown,
                result.message.clone(),
            ),
            None => (
                EngineHealthStatus::VerificationRequired,
                EngineAuthState::Unknown,
                "请先在应用设置中运行 Grok Build 运行时自检".to_string(),
            ),
        }
    };
    EngineHealth {
        runtime: ExecutionRuntime::BuiltIn,
        provider: ExecutionProvider::GrokBuild,
        status,
        executable_path: None,
        version: Some(format!("adapter-v{}", metheus_grok_engine::ADAPTER_VERSION)),
        auth_state,
        authentication: EngineAuthenticationResult {
            local_state: if secret_configured {
                EngineLocalAuthState::ConfiguredEvidence
            } else {
                EngineLocalAuthState::Missing
            },
            online_state: if self_test.as_ref().is_some_and(|result| result.success) {
                EngineOnlineAuthState::Verified
            } else if self_test.is_some() {
                EngineOnlineAuthState::Failed
            } else {
                EngineOnlineAuthState::NotVerified
            },
            method: EngineAuthVerificationMethod::OnlineMinimalRequest,
            verified_at: self_test.as_ref().map(|result| result.verified_at.clone()),
            expires_at: None,
            failure_kind: None,
            runtime_configuration: Some(super::health::builtin_runtime_configuration_evidence(
                settings,
            )),
            message: message.clone(),
        },
        supports_unattended: true,
        configuration_valid: true,
        capabilities: vec![
            "in-process".to_string(),
            "project-read".to_string(),
            "authorized-file-write".to_string(),
            "no-shell".to_string(),
            "no-subagents".to_string(),
        ],
        source_revision: Some(source_revision),
        runtime_self_test: self_test_state,
        message,
    }
}

pub(crate) async fn test_runtime() -> EngineRuntimeSelfTestResult {
    let verified_at = chrono::Utc::now().to_rfc3339();
    let source_revision = metheus_grok_engine::source_revision().to_string();
    let snapshot = match crate::settings::begin_built_in_grok_build_request() {
        Ok(snapshot) => snapshot,
        Err(message) => {
            return EngineRuntimeSelfTestResult {
                success: false,
                state: EngineRuntimeSelfTestState::Failed,
                source_revision,
                verified_at,
                message,
            };
        }
    };
    let settings = AppSettings {
        revision: snapshot.settings_revision,
        built_in_grok_build: snapshot.settings.clone(),
        ..AppSettings::default()
    };
    let result = metheus_grok_engine::run_runtime_self_test(adapter_config(
        &snapshot.settings,
        &snapshot.api_key,
        snapshot.settings.max_turns,
        2,
        0,
    ))
    .await;
    let result = match result {
        Ok(_) => EngineRuntimeSelfTestResult {
            success: true,
            state: EngineRuntimeSelfTestState::Passed,
            source_revision,
            verified_at,
            message: "Grok Build 运行时自检通过".to_string(),
        },
        Err(error) => EngineRuntimeSelfTestResult {
            success: false,
            state: EngineRuntimeSelfTestState::Failed,
            source_revision,
            verified_at,
            message: error.message().to_string(),
        },
    };
    cache_self_test(&settings, &snapshot.api_key, result.clone());
    result
}

pub(super) async fn test_model_connection() -> ConnectionTestResult {
    let started = std::time::Instant::now();
    let snapshot = match crate::settings::begin_built_in_grok_build_request() {
        Ok(snapshot) => snapshot,
        Err(message) => {
            return ConnectionTestResult {
                success: false,
                target: ModelConnectionTarget::BuiltInGrokBuild,
                model: String::new(),
                latency_ms: elapsed_millis(started),
                error_kind: Some(ModelConnectionErrorKind::MissingSecret),
                message,
            };
        }
    };
    let result = metheus_grok_engine::test_model_connection(adapter_config(
        &snapshot.settings,
        &snapshot.api_key,
        snapshot.settings.max_turns,
        0,
        0,
    ))
    .await;
    ConnectionTestResult {
        success: result.success,
        target: ModelConnectionTarget::BuiltInGrokBuild,
        model: result.model,
        latency_ms: result.latency_ms,
        error_kind: result.error_kind.map(map_connection_error),
        message: result.message,
    }
}

fn map_connection_error(kind: GrokBuildRuntimeErrorKind) -> ModelConnectionErrorKind {
    match kind {
        GrokBuildRuntimeErrorKind::InvalidConfiguration => {
            ModelConnectionErrorKind::InvalidConfiguration
        }
        GrokBuildRuntimeErrorKind::Authentication => ModelConnectionErrorKind::Authentication,
        GrokBuildRuntimeErrorKind::QuotaExceeded => ModelConnectionErrorKind::QuotaExceeded,
        GrokBuildRuntimeErrorKind::RateLimited => ModelConnectionErrorKind::RateLimited,
        GrokBuildRuntimeErrorKind::Network => ModelConnectionErrorKind::Network,
        GrokBuildRuntimeErrorKind::ProviderUnavailable => {
            ModelConnectionErrorKind::ProviderUnavailable
        }
        GrokBuildRuntimeErrorKind::Timeout => ModelConnectionErrorKind::Timeout,
        GrokBuildRuntimeErrorKind::Cancelled
        | GrokBuildRuntimeErrorKind::ToolRejected
        | GrokBuildRuntimeErrorKind::ToolFailed
        | GrokBuildRuntimeErrorKind::Protocol
        | GrokBuildRuntimeErrorKind::OutputTruncated
        | GrokBuildRuntimeErrorKind::MaxTurns
        | GrokBuildRuntimeErrorKind::Runtime => ModelConnectionErrorKind::Protocol,
    }
}

fn elapsed_millis(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn map_failure_kind(kind: GrokBuildRuntimeErrorKind) -> EngineFailureKind {
    match kind {
        GrokBuildRuntimeErrorKind::Authentication => EngineFailureKind::AuthenticationError,
        GrokBuildRuntimeErrorKind::QuotaExceeded => EngineFailureKind::QuotaExceeded,
        GrokBuildRuntimeErrorKind::RateLimited => EngineFailureKind::RateLimited,
        GrokBuildRuntimeErrorKind::Network => EngineFailureKind::NetworkError,
        GrokBuildRuntimeErrorKind::ProviderUnavailable => EngineFailureKind::ProviderUnavailable,
        GrokBuildRuntimeErrorKind::Timeout => EngineFailureKind::Timeout,
        GrokBuildRuntimeErrorKind::ToolRejected => EngineFailureKind::ToolRejected,
        GrokBuildRuntimeErrorKind::Protocol => EngineFailureKind::ProtocolError,
        GrokBuildRuntimeErrorKind::OutputTruncated => EngineFailureKind::OutputTruncated,
        GrokBuildRuntimeErrorKind::MaxTurns => EngineFailureKind::MaxTurnsExceeded,
        GrokBuildRuntimeErrorKind::InvalidConfiguration | GrokBuildRuntimeErrorKind::Runtime => {
            EngineFailureKind::RuntimeError
        }
        GrokBuildRuntimeErrorKind::ToolFailed | GrokBuildRuntimeErrorKind::Cancelled => {
            EngineFailureKind::TaskExecutionError
        }
    }
}

fn runtime_event_log(event: GrokBuildRuntimeEvent) -> (&'static str, String) {
    match event {
        GrokBuildRuntimeEvent::Started { source_revision } => {
            let short_revision = source_revision.get(..8).unwrap_or(&source_revision);
            (
                "info",
                format!("[Grok Build 内置] 运行时启动 · 源码 {short_revision}"),
            )
        }
        GrokBuildRuntimeEvent::ModelText { text } => ("info", format!("[Grok Build 内置] {text}")),
        GrokBuildRuntimeEvent::ToolStarted { name } => {
            ("info", format!("[Grok Build 内置] 调用工具 {name}"))
        }
        GrokBuildRuntimeEvent::ToolCompleted { name, .. } => {
            ("info", format!("[Grok Build 内置] 工具 {name} 已完成"))
        }
        GrokBuildRuntimeEvent::ToolFailed { name, .. } => {
            ("error", format!("[Grok Build 内置] 工具 {name} 执行失败"))
        }
        GrokBuildRuntimeEvent::RetryScheduled {
            attempt,
            max_retries,
            reason,
        } => (
            "warn",
            format!("[Grok Build 内置] 正在重试 {attempt}/{max_retries} · {reason}"),
        ),
        GrokBuildRuntimeEvent::RetryExhausted {
            attempts,
            reason,
            is_rate_limited,
        } => (
            "error",
            format!(
                "[Grok Build 内置] 重试已耗尽 · {attempts} 次{} · {reason}",
                if is_rate_limited { " · 限流" } else { "" }
            ),
        ),
        GrokBuildRuntimeEvent::RetryFailed {
            error_type,
            message,
        } => (
            "error",
            format!("[Grok Build 内置] 不可重试错误 · {error_type} · {message}"),
        ),
        GrokBuildRuntimeEvent::TokenUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
        } => (
            "info",
            format!(
                "[Grok Build 内置] Token · 输入 {prompt_tokens} · 输出 {completion_tokens} · 总计 {total_tokens}"
            ),
        ),
        GrokBuildRuntimeEvent::Completed {
            turns,
            files_written,
        } => (
            "info",
            format!("[Grok Build 内置] 完成 · {turns} 轮 · 写入 {files_written} 个授权文件"),
        ),
    }
}

fn event_sink(
    state: Arc<AsyncMutex<Option<PipelineState>>>,
    execution_id: String,
) -> RuntimeEventSink {
    RuntimeEventSink::new(move |event| {
        let project_name = {
            let mut guard = state.blocking_lock();
            let Some(pipeline) = guard.as_mut() else {
                return;
            };
            if pipeline.execution_id != execution_id || pipeline.status != PipelineStatus::Running {
                return;
            }
            let (level, text) = runtime_event_log(event);
            if text.trim().is_empty() {
                return;
            }
            append_runtime_log(pipeline, level, text);
            pipeline.project_name.clone()
        };
        if !project_name.is_empty() {
            let _ = crate::project_state_bus::publish_project_runtime_state(&project_name);
        }
    })
}

fn provider_usage(usage: Option<TokenUsage>) -> Option<crate::cost_ledger::ProviderUsage> {
    usage.map(|usage| crate::cost_ledger::ProviderUsage {
        input_tokens: Some(usage.prompt_tokens),
        output_tokens: Some(usage.completion_tokens),
        total_tokens: Some(usage.total_tokens),
        cached_input_tokens: None,
    })
}

fn merge_file_facts(mut detected: Vec<String>, runtime: Vec<String>) -> Vec<String> {
    for path in runtime {
        if !detected.contains(&path) {
            detected.push(path);
        }
    }
    detected
}

#[allow(clippy::too_many_arguments)]
fn runtime_failure_result(
    app_settings: &AppSettings,
    kind: GrokBuildRuntimeErrorKind,
    message: String,
    output_summary: String,
    token_usage: Option<TokenUsage>,
    runtime_files: Vec<String>,
    detected_files: Vec<String>,
) -> ExecutionResult {
    ExecutionResult {
        success: false,
        output: output_summary.clone(),
        error_log: message.clone(),
        file_changes: merge_file_facts(detected_files, runtime_files),
        exit_code: None,
        engine_provider: Some(ExecutionProvider::GrokBuild),
        engine_runtime: ExecutionRuntime::BuiltIn,
        engine_settings_revision: app_settings.revision,
        engine_source_revision: metheus_grok_engine::source_revision().to_string(),
        engine_api_backend: app_settings
            .built_in_grok_build
            .api_backend
            .as_str()
            .to_string(),
        stdout: output_summary,
        stderr: message,
        engine_failure_kind: Some(map_failure_kind(kind)),
        token_usage: provider_usage(token_usage),
    }
}

fn interrupted_engine_error(
    kind: GrokBuildRuntimeErrorKind,
    result: ExecutionResult,
    stop_reason: Option<ExecutionStopReason>,
    resource_observation: crate::project::ResourceObservationSummary,
) -> Result<ExecutionResult, EngineError> {
    match kind {
        GrokBuildRuntimeErrorKind::Cancelled => match stop_reason {
            Some(ExecutionStopReason::ResourceHardStop) => {
                Err(EngineError::resource_hard_stop_with_result_and_observation(
                    result,
                    resource_observation,
                ))
            }
            Some(ExecutionStopReason::WallClockExceeded) => {
                Err(EngineError::timeout_with_result(result))
            }
            _ => Err(EngineError::cancelled_with_result(result)),
        },
        GrokBuildRuntimeErrorKind::Timeout => Err(EngineError::timeout_with_result(result)),
        _ => Ok(result),
    }
}

fn sample_builtin_resource(
    guard: &ExecutionGuard,
    sampler: impl FnOnce(Option<&str>) -> crate::runtime_resource::ResourceObservation,
) {
    let sampled_at = chrono::Utc::now().to_rfc3339();
    let sample = sampler(Some(sampled_at.as_str()));
    guard.set_resource_observation(sample.summary);
    guard.set_resource_decision(sample.decision);
}

pub(super) async fn execute(
    app_settings: &AppSettings,
    api_key: &str,
    request: ExecutionRequest,
    state: Arc<AsyncMutex<Option<PipelineState>>>,
) -> Result<ExecutionResult, EngineError> {
    let before_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let guard = ExecutionGuard::new(&request.task_budget, ResourceDecision::Unknown);
    sample_builtin_resource(&guard, crate::runtime_resource::observe_current_process);
    if guard.stop_reason() == Some(ExecutionStopReason::ResourceHardStop) {
        let result = runtime_failure_result(
            app_settings,
            GrokBuildRuntimeErrorKind::Runtime,
            "内置执行启动前资源已达到硬停止阈值".to_string(),
            String::new(),
            None,
            vec![],
            vec![],
        );
        return Err(EngineError::resource_hard_stop_with_result_and_observation(
            result,
            guard.resource_observation(),
        ));
    }
    let cancellation = guard.cancellation();
    let monitor_guard = guard.clone();
    let monitor_state = state.clone();
    let monitor_execution_id = request.execution_id.clone();
    let monitor = tokio::spawn(async move {
        loop {
            sample_builtin_resource(
                &monitor_guard,
                crate::runtime_resource::observe_current_process,
            );
            let should_cancel = {
                let guard = monitor_state.lock().await;
                guard.as_ref().is_none_or(|pipeline| {
                    pipeline.execution_id != monitor_execution_id
                        || matches!(
                            pipeline.status,
                            PipelineStatus::Paused | PipelineStatus::Failed
                        )
                })
            };
            if should_cancel {
                monitor_guard.request_cancel();
                break;
            }
            if matches!(
                monitor_guard.stop_reason(),
                Some(
                    ExecutionStopReason::WallClockExceeded | ExecutionStopReason::ResourceHardStop
                )
            ) {
                monitor_guard.request_cancel();
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    });
    let adapter_request = GrokBuildExecutionRequest {
        project_path: PathBuf::from(&request.project_path),
        prompt: request.prompt.clone(),
        authorized_paths: request.authorized_paths.iter().map(PathBuf::from).collect(),
        execution_id: request.execution_id.clone(),
        cancellation,
        event_sink: Some(event_sink(state, request.execution_id.clone())),
    };
    let (wall_clock_expired, result) = tokio::select! {
        result = metheus_grok_engine::execute(
            adapter_config(
                &app_settings.built_in_grok_build,
                api_key,
                app_settings
                    .built_in_grok_build
                    .max_turns
                    .min(request.task_budget.max_executor_turns),
                request.task_budget.max_transport_retries,
                request.task_budget.max_doom_loop_retries,
            ),
            adapter_request,
        ) => (false, Some(result)),
        _ = tokio::time::sleep(guard.remaining()) => {
            guard.request_cancel();
            (true, None)
        }
    };
    monitor.abort();
    let after_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let file_changes =
        crate::test_runner::detect_changes(&before_files, &after_files, &request.project_path);
    if wall_clock_expired {
        let stop_reason = guard
            .stop_reason()
            .unwrap_or(ExecutionStopReason::WallClockExceeded);
        let result = runtime_failure_result(
            app_settings,
            if stop_reason == ExecutionStopReason::ResourceHardStop {
                GrokBuildRuntimeErrorKind::Runtime
            } else {
                GrokBuildRuntimeErrorKind::Timeout
            },
            if stop_reason == ExecutionStopReason::ResourceHardStop {
                "内置执行因资源压力达到硬停止阈值而终止".to_string()
            } else {
                format!(
                    "内置执行达到任务总墙钟上限（{} 秒）",
                    request.task_budget.bounded_wall_clock_secs()
                )
            },
            String::new(),
            None,
            vec![],
            file_changes,
        );
        return if stop_reason == ExecutionStopReason::ResourceHardStop {
            Err(EngineError::resource_hard_stop_with_result_and_observation(
                result,
                guard.resource_observation(),
            ))
        } else {
            Err(EngineError::timeout_with_result(result))
        };
    }
    let result = result.expect("内置执行 future 未在完成或总墙钟分支中返回");
    match result {
        Ok(result) => {
            if let Some(stop_reason) = guard.stop_reason() {
                let (kind, message) = match stop_reason {
                    ExecutionStopReason::ResourceHardStop => (
                        GrokBuildRuntimeErrorKind::Runtime,
                        "内置执行因资源压力达到硬停止阈值而终止".to_string(),
                    ),
                    ExecutionStopReason::WallClockExceeded => (
                        GrokBuildRuntimeErrorKind::Timeout,
                        format!(
                            "内置执行达到任务总墙钟上限（{} 秒）",
                            request.task_budget.bounded_wall_clock_secs()
                        ),
                    ),
                    ExecutionStopReason::Cancelled => (
                        GrokBuildRuntimeErrorKind::Cancelled,
                        "内置执行已暂停".to_string(),
                    ),
                };
                let failure_result = runtime_failure_result(
                    app_settings,
                    kind,
                    message,
                    result.output,
                    result.token_usage,
                    result.files_written,
                    file_changes,
                );
                return match stop_reason {
                    ExecutionStopReason::ResourceHardStop => {
                        Err(EngineError::resource_hard_stop_with_result_and_observation(
                            failure_result,
                            guard.resource_observation(),
                        ))
                    }
                    ExecutionStopReason::WallClockExceeded => {
                        Err(EngineError::timeout_with_result(failure_result))
                    }
                    ExecutionStopReason::Cancelled => {
                        Err(EngineError::cancelled_with_result(failure_result))
                    }
                };
            }
            let output = result.output;
            let token_usage = provider_usage(result.token_usage);
            Ok(ExecutionResult {
                success: true,
                output: output.clone(),
                error_log: String::new(),
                file_changes: merge_file_facts(file_changes, result.files_written),
                exit_code: None,
                engine_provider: Some(ExecutionProvider::GrokBuild),
                engine_runtime: ExecutionRuntime::BuiltIn,
                engine_settings_revision: app_settings.revision,
                engine_source_revision: metheus_grok_engine::source_revision().to_string(),
                engine_api_backend: app_settings
                    .built_in_grok_build
                    .api_backend
                    .as_str()
                    .to_string(),
                stdout: output,
                stderr: String::new(),
                engine_failure_kind: None,
                token_usage,
            })
        }
        Err(error) => {
            let kind = error.kind;
            let message = error.message().to_string();
            let result = runtime_failure_result(
                app_settings,
                kind,
                message,
                error.output_summary,
                error.token_usage,
                error.files_written,
                file_changes,
            );
            interrupted_engine_error(
                kind,
                result,
                guard.stop_reason(),
                guard.resource_observation(),
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource_sample(
        decision: ResourceDecision,
        state: crate::project::ResourceObservationState,
    ) -> crate::runtime_resource::ResourceObservation {
        crate::runtime_resource::ResourceObservation {
            summary: crate::project::ResourceObservationSummary {
                state,
                ..Default::default()
            },
            decision,
        }
    }

    #[test]
    fn builtin_production_sampler_updates_warning_and_hard_stop_guard() {
        let guard = ExecutionGuard::new(
            &crate::task_contract::TaskBudgetSummary::default(),
            ResourceDecision::Unknown,
        );
        sample_builtin_resource(&guard, |_| {
            resource_sample(
                ResourceDecision::Warning,
                crate::project::ResourceObservationState::Warning,
            )
        });
        assert_eq!(
            guard.resource_observation().state,
            crate::project::ResourceObservationState::Warning
        );
        assert_eq!(guard.stop_reason(), None);

        sample_builtin_resource(&guard, |_| {
            resource_sample(
                ResourceDecision::HardStop,
                crate::project::ResourceObservationState::HardStop,
            )
        });
        assert_eq!(
            guard.stop_reason(),
            Some(ExecutionStopReason::ResourceHardStop)
        );
    }

    #[test]
    fn builtin_runtime_reports_missing_secret_without_path_lookup() {
        let health = health(&AppSettings::default(), None);
        assert_eq!(health.status, EngineHealthStatus::Unauthenticated);
        assert!(health.configuration_valid);
        assert!(health.executable_path.is_none());
        assert_eq!(
            health.source_revision.as_deref(),
            Some(metheus_grok_engine::source_revision())
        );
    }

    #[test]
    fn builtin_health_reflects_self_test_state() {
        let settings = AppSettings::default();
        let api_key = "self-test-state-secret";
        if let Ok(mut cache) = SELF_TEST_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = None;
        }

        let not_run = health(&settings, Some(api_key));
        assert_eq!(not_run.status, EngineHealthStatus::VerificationRequired);
        assert_eq!(
            not_run.runtime_self_test,
            EngineRuntimeSelfTestState::NotRun
        );
        assert_eq!(
            not_run
                .authentication
                .runtime_configuration
                .as_ref()
                .and_then(|evidence| evidence.model.as_deref()),
            Some(settings.built_in_grok_build.model.as_str())
        );

        cache_self_test(
            &settings,
            api_key,
            EngineRuntimeSelfTestResult {
                success: true,
                state: EngineRuntimeSelfTestState::Passed,
                source_revision: metheus_grok_engine::source_revision().to_string(),
                verified_at: "2026-08-10T00:00:00Z".to_string(),
                message: "passed".to_string(),
            },
        );
        let passed = health(&settings, Some(api_key));
        assert_eq!(passed.status, EngineHealthStatus::Available);
        assert_eq!(passed.runtime_self_test, EngineRuntimeSelfTestState::Passed);

        cache_self_test(
            &settings,
            api_key,
            EngineRuntimeSelfTestResult {
                success: false,
                state: EngineRuntimeSelfTestState::Failed,
                source_revision: metheus_grok_engine::source_revision().to_string(),
                verified_at: "2026-08-10T00:00:01Z".to_string(),
                message: "failed".to_string(),
            },
        );
        let failed = health(&settings, Some(api_key));
        assert_eq!(failed.status, EngineHealthStatus::VerificationFailed);
        assert_eq!(failed.runtime_self_test, EngineRuntimeSelfTestState::Failed);

        if let Ok(mut cache) = SELF_TEST_CACHE.get_or_init(|| Mutex::new(None)).lock() {
            *cache = None;
        }
    }

    #[test]
    fn self_test_identity_changes_with_secret_and_runtime_settings() {
        let settings = AppSettings::default();
        let base = SelfTestCacheIdentity::new(&settings, "first-secret");
        assert!(base != SelfTestCacheIdentity::new(&settings, "second-secret"));

        let mut changed = settings.clone();
        changed.revision += 1;
        assert!(base != SelfTestCacheIdentity::new(&changed, "first-secret"));
        changed = settings.clone();
        changed.built_in_grok_build.timeout_secs += 1;
        assert!(base != SelfTestCacheIdentity::new(&changed, "first-secret"));
        changed = settings.clone();
        changed.built_in_grok_build.max_turns += 1;
        assert!(base != SelfTestCacheIdentity::new(&changed, "first-secret"));
    }

    #[test]
    fn adaptive_grok_contract_task_budget_never_exceeds_user_setting() {
        let mut settings = BuiltInGrokBuildSettings::default();
        settings.max_turns = 12;
        assert_eq!(adapter_config(&settings, "secret", 4, 1, 0).max_turns, 4);
        assert_eq!(
            adapter_config(&settings, "secret", settings.max_turns.min(32), 3, 2).max_turns,
            12
        );
        let mapped = adapter_config(&settings, "secret", 8, 3, 2);
        assert_eq!(mapped.max_transport_retries, 3);
        assert_eq!(mapped.max_doom_loop_retries, 2);
    }

    #[test]
    fn adaptive_grok_contract_events_use_failure_aware_log_levels() {
        let (level, text) = runtime_event_log(GrokBuildRuntimeEvent::ToolFailed {
            name: "search_replace".into(),
            summary: "rejected".into(),
        });
        assert_eq!(level, "error");
        assert!(text.contains("执行失败"));
        assert!(!text.contains("已完成"));

        let (level, text) = runtime_event_log(GrokBuildRuntimeEvent::RetryScheduled {
            attempt: 1,
            max_retries: 3,
            reason: "service unavailable".into(),
        });
        assert_eq!(level, "warn");
        assert!(text.contains("1/3"));

        let (level, _) = runtime_event_log(GrokBuildRuntimeEvent::RetryExhausted {
            attempts: 4,
            reason: "service unavailable".into(),
            is_rate_limited: false,
        });
        assert_eq!(level, "error");
    }

    #[test]
    fn adaptive_grok_contract_errors_map_without_text_inference() {
        let cases = [
            (
                GrokBuildRuntimeErrorKind::Authentication,
                EngineFailureKind::AuthenticationError,
            ),
            (
                GrokBuildRuntimeErrorKind::QuotaExceeded,
                EngineFailureKind::QuotaExceeded,
            ),
            (
                GrokBuildRuntimeErrorKind::RateLimited,
                EngineFailureKind::RateLimited,
            ),
            (
                GrokBuildRuntimeErrorKind::ProviderUnavailable,
                EngineFailureKind::ProviderUnavailable,
            ),
            (
                GrokBuildRuntimeErrorKind::Network,
                EngineFailureKind::NetworkError,
            ),
            (
                GrokBuildRuntimeErrorKind::Timeout,
                EngineFailureKind::Timeout,
            ),
            (
                GrokBuildRuntimeErrorKind::ToolRejected,
                EngineFailureKind::ToolRejected,
            ),
            (
                GrokBuildRuntimeErrorKind::Protocol,
                EngineFailureKind::ProtocolError,
            ),
            (
                GrokBuildRuntimeErrorKind::OutputTruncated,
                EngineFailureKind::OutputTruncated,
            ),
            (
                GrokBuildRuntimeErrorKind::MaxTurns,
                EngineFailureKind::MaxTurnsExceeded,
            ),
            (
                GrokBuildRuntimeErrorKind::InvalidConfiguration,
                EngineFailureKind::RuntimeError,
            ),
            (
                GrokBuildRuntimeErrorKind::Runtime,
                EngineFailureKind::RuntimeError,
            ),
            (
                GrokBuildRuntimeErrorKind::ToolFailed,
                EngineFailureKind::TaskExecutionError,
            ),
            (
                GrokBuildRuntimeErrorKind::Cancelled,
                EngineFailureKind::TaskExecutionError,
            ),
        ];
        for (source, expected) in cases {
            assert_eq!(map_failure_kind(source), expected);
        }
    }

    #[test]
    fn adaptive_grok_contract_truncation_wrapper_preserves_structured_facts() {
        let result = runtime_failure_result(
            &AppSettings::default(),
            GrokBuildRuntimeErrorKind::OutputTruncated,
            "max tokens reached".to_string(),
            "partial output".to_string(),
            Some(TokenUsage {
                prompt_tokens: 9,
                completion_tokens: 6,
                total_tokens: 15,
            }),
            vec!["runtime.txt".to_string()],
            vec!["detected.txt".to_string(), "runtime.txt".to_string()],
        );

        assert!(!result.success);
        assert_eq!(
            result.engine_failure_kind,
            Some(EngineFailureKind::OutputTruncated)
        );
        assert_eq!(result.engine_runtime, ExecutionRuntime::BuiltIn);
        assert_eq!(result.output, "partial output");
        assert_eq!(result.file_changes, vec!["detected.txt", "runtime.txt"]);
        assert_eq!(
            result.token_usage,
            Some(crate::cost_ledger::ProviderUsage {
                input_tokens: Some(9),
                output_tokens: Some(6),
                total_tokens: Some(15),
                cached_input_tokens: None,
            })
        );
    }

    #[test]
    fn adaptive_grok_contract_interruptions_preserve_usage_files_and_output() {
        for (kind, expected_failure) in [
            (
                GrokBuildRuntimeErrorKind::Timeout,
                EngineFailureKind::Timeout,
            ),
            (
                GrokBuildRuntimeErrorKind::Cancelled,
                EngineFailureKind::TaskExecutionError,
            ),
        ] {
            let result = runtime_failure_result(
                &AppSettings::default(),
                kind,
                format!("{kind:?}"),
                "first attempt\ncontinuation".to_string(),
                Some(TokenUsage {
                    prompt_tokens: 11,
                    completion_tokens: 7,
                    total_tokens: 18,
                }),
                vec!["shared.txt".to_string(), "runtime.txt".to_string()],
                vec!["detected.txt".to_string(), "shared.txt".to_string()],
            );
            let error = interrupted_engine_error(
                kind,
                result,
                None,
                crate::project::ResourceObservationSummary::default(),
            )
            .unwrap_err();
            let partial = match error {
                EngineError::Timeout { execution_result }
                | EngineError::Cancelled { execution_result } => {
                    execution_result.expect("BuiltIn interruption must retain execution facts")
                }
                other => panic!("unexpected interruption mapping: {other:?}"),
            };
            assert_eq!(partial.engine_failure_kind, Some(expected_failure));
            assert_eq!(partial.output, "first attempt\ncontinuation");
            assert_eq!(partial.stdout, partial.output);
            assert_eq!(
                partial.file_changes,
                vec!["detected.txt", "shared.txt", "runtime.txt"]
            );
            assert_eq!(
                partial.token_usage,
                Some(crate::cost_ledger::ProviderUsage {
                    input_tokens: Some(11),
                    output_tokens: Some(7),
                    total_tokens: Some(18),
                    cached_input_tokens: None,
                })
            );
        }
    }
}
