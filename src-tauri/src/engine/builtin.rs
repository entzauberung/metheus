use super::contract::{
    EngineAuthState, EngineAuthVerificationMethod, EngineAuthenticationResult, EngineError,
    EngineHealth, EngineHealthStatus, EngineLocalAuthState, EngineOnlineAuthState,
    EngineRuntimeSelfTestResult, EngineRuntimeSelfTestState, ExecutionRequest,
};
use crate::pipeline::{append_runtime_log, PipelineState, PipelineStatus};
use crate::project::{EngineFailureKind, ExecutionProvider, ExecutionResult, ExecutionRuntime};
use crate::settings::{AppSettings, BuiltInGrokBuildSettings, GrokBuildApiBackend};
use metheus_grok_engine::{
    GrokBuildExecutionConfig, GrokBuildExecutionRequest, GrokBuildRuntimeErrorKind,
    GrokBuildRuntimeEvent, RuntimeEventSink,
};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
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

fn adapter_config(settings: &BuiltInGrokBuildSettings, api_key: &str) -> GrokBuildExecutionConfig {
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
        max_turns: settings.max_turns,
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
            }
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

fn map_failure_kind(kind: GrokBuildRuntimeErrorKind) -> EngineFailureKind {
    match kind {
        GrokBuildRuntimeErrorKind::Authentication => EngineFailureKind::AuthenticationError,
        GrokBuildRuntimeErrorKind::QuotaExceeded => EngineFailureKind::QuotaExceeded,
        GrokBuildRuntimeErrorKind::RateLimited => EngineFailureKind::RateLimited,
        GrokBuildRuntimeErrorKind::Network => EngineFailureKind::NetworkError,
        GrokBuildRuntimeErrorKind::ProviderUnavailable => EngineFailureKind::ProviderUnavailable,
        GrokBuildRuntimeErrorKind::Timeout => EngineFailureKind::Timeout,
        GrokBuildRuntimeErrorKind::InvalidConfiguration
        | GrokBuildRuntimeErrorKind::ToolRejected
        | GrokBuildRuntimeErrorKind::ToolFailed
        | GrokBuildRuntimeErrorKind::Protocol
        | GrokBuildRuntimeErrorKind::MaxTurns
        | GrokBuildRuntimeErrorKind::Runtime
        | GrokBuildRuntimeErrorKind::Cancelled => EngineFailureKind::TaskExecutionError,
    }
}

fn event_sink(
    state: Arc<AsyncMutex<Option<PipelineState>>>,
    execution_id: String,
) -> RuntimeEventSink {
    RuntimeEventSink::new(move |event| {
        let Ok(mut guard) = state.try_lock() else {
            return;
        };
        let Some(pipeline) = guard.as_mut() else {
            return;
        };
        if pipeline.execution_id != execution_id || pipeline.status != PipelineStatus::Running {
            return;
        }
        let text = match event {
            GrokBuildRuntimeEvent::Started { source_revision } => {
                format!(
                    "[Grok Build 内置] 运行时启动 · 源码 {}",
                    &source_revision[..8]
                )
            }
            GrokBuildRuntimeEvent::ModelText { text } => {
                let display: String = text.chars().take(2_000).collect();
                format!("[Grok Build 内置] {}", display.trim())
            }
            GrokBuildRuntimeEvent::ToolStarted { name } => {
                format!("[Grok Build 内置] 调用工具 {name}")
            }
            GrokBuildRuntimeEvent::ToolCompleted { name, .. } => {
                format!("[Grok Build 内置] 工具 {name} 已完成")
            }
            GrokBuildRuntimeEvent::Completed {
                turns,
                files_written,
            } => format!("[Grok Build 内置] 完成 · {turns} 轮 · 写入 {files_written} 个授权文件"),
        };
        if !text.trim().is_empty() {
            append_runtime_log(pipeline, "info", text);
        }
    })
}

pub(super) async fn execute(
    app_settings: &AppSettings,
    api_key: &str,
    request: ExecutionRequest,
    state: Arc<AsyncMutex<Option<PipelineState>>>,
) -> Result<ExecutionResult, EngineError> {
    let before_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let cancellation = Arc::new(AtomicBool::new(false));
    let monitor_cancellation = cancellation.clone();
    let monitor_state = state.clone();
    let monitor_execution_id = request.execution_id.clone();
    let monitor = tokio::spawn(async move {
        loop {
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
                monitor_cancellation.store(true, Ordering::Relaxed);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
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
    let result = metheus_grok_engine::execute(
        adapter_config(&app_settings.built_in_grok_build, api_key),
        adapter_request,
    )
    .await;
    monitor.abort();
    let after_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let file_changes =
        crate::test_runner::detect_changes(&before_files, &after_files, &request.project_path);
    match result {
        Ok(result) => {
            let output = result.output;
            Ok(ExecutionResult {
                success: true,
                output: output.clone(),
                error_log: String::new(),
                file_changes,
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
            })
        }
        Err(error) if error.kind == GrokBuildRuntimeErrorKind::Cancelled => {
            Err(EngineError::Cancelled)
        }
        Err(error) if error.kind == GrokBuildRuntimeErrorKind::Timeout => Err(EngineError::Timeout),
        Err(error) if error.kind == GrokBuildRuntimeErrorKind::InvalidConfiguration => Err(
            EngineError::InvalidConfiguration(error.message().to_string()),
        ),
        Err(error) => {
            let message = error.message().to_string();
            Ok(ExecutionResult {
                success: false,
                output: String::new(),
                error_log: message.clone(),
                file_changes,
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
                stdout: String::new(),
                stderr: message,
                engine_failure_kind: Some(map_failure_kind(error.kind)),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
