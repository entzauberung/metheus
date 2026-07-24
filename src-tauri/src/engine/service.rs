use super::contract::{EngineError, EngineHealth, ExecutionRequest, ProgramSource};
use super::health::HealthCheckResult;
use crate::pipeline::PipelineState;
use crate::project::{
    ExecutionProfile, ExecutionProvider, ExecutionResult, ExecutionRuntime, PermissionProfile,
};
use crate::settings::EngineOperationSnapshot;
use std::ffi::OsString;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct PreparedEngine {
    profile: ExecutionProfile,
    pub(crate) health: EngineHealth,
    operation: EngineOperationSnapshot,
    program: Option<OsString>,
    program_source: Option<ProgramSource>,
}

impl PreparedEngine {
    pub(crate) fn settings(&self) -> &crate::settings::AppSettings {
        &self.operation.settings
    }
}

pub(crate) fn validate_profile(profile: &ExecutionProfile) -> Result<(), String> {
    match (&profile.runtime, &profile.provider) {
        (
            ExecutionRuntime::Plugin,
            ExecutionProvider::ClaudeCode
            | ExecutionProvider::Codex
            | ExecutionProvider::KimiCli
            | ExecutionProvider::GrokBuild,
        ) => {}
        (ExecutionRuntime::BuiltIn, ExecutionProvider::GrokBuild) => {}
        _ => return Err("执行模式与引擎组合无效".to_string()),
    }
    if profile.permission_profile != PermissionProfile::Unattended {
        return Err("当前后台流水线只支持 Unattended 权限模式".to_string());
    }
    Ok(())
}

pub(crate) async fn prepare_engine(profile: &ExecutionProfile) -> Result<PreparedEngine, String> {
    validate_profile(profile)?;
    let operation = crate::settings::begin_engine_operation()?;
    let HealthCheckResult {
        health,
        program,
        program_source,
    } = super::health::check_engine_health_with_settings(
        profile,
        &operation.settings,
        operation.built_in_grok_build_api_key.as_deref(),
    )
    .await;
    Ok(PreparedEngine {
        profile: profile.clone(),
        health,
        operation,
        program,
        program_source,
    })
}

pub(crate) async fn check_engine_health(profile: &ExecutionProfile) -> EngineHealth {
    match prepare_engine(profile).await {
        Ok(prepared) => prepared.health,
        Err(message) => super::health::settings_failure(profile, message),
    }
}

pub(crate) async fn verify_engine_authentication(
    profile: &ExecutionProfile,
) -> Result<super::contract::EngineAuthenticationResult, String> {
    validate_profile(profile)?;
    let operation = crate::settings::begin_engine_operation()?;
    super::health::verify_engine_authentication_with_settings(profile, &operation.settings).await
}

pub(crate) async fn execute(
    prepared: PreparedEngine,
    request: ExecutionRequest,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<ExecutionResult, EngineError> {
    let profile = &prepared.profile;
    if prepared.health.status.blocks_execution() {
        let message = format!("执行引擎不可用：{}", prepared.health.message);
        return if prepared.health.configuration_valid {
            Err(EngineError::Unavailable(message))
        } else {
            Err(EngineError::InvalidConfiguration(message))
        };
    }
    if profile.runtime == ExecutionRuntime::BuiltIn {
        let api_key = prepared
            .operation
            .built_in_grok_build_api_key
            .as_deref()
            .ok_or_else(|| {
                EngineError::InvalidConfiguration(
                    "预装 Grok Build API Key 未配置或安全凭据库不可用".to_string(),
                )
            })?;
        return super::builtin::execute(&prepared.operation.settings, api_key, request, state)
            .await;
    }

    let full_prompt = format!(
        "{}\n\n=== V1 执行约束 ===\n允许新增、修改或删除的精确文件路径：\n- {}\n\
         1. 只执行上述任务，只能变更列出的精确文件，不得扩展到目录、相邻文件或改变架构。\n\
         2. 信息不足或发现范围外问题时，必须停止并说明阻塞原因，不得自行猜测或扩展。\n\
         3. 完成后不要输出总结，直接结束。",
        request.prompt,
        request.authorized_paths.join("\n- ")
    );
    let program = prepared.program.ok_or_else(|| {
        EngineError::InvalidConfiguration("健康检查未解析出执行引擎程序路径".to_string())
    })?;
    let program_source = prepared.program_source.ok_or_else(|| {
        EngineError::InvalidConfiguration("健康检查未记录执行引擎程序来源".to_string())
    })?;
    let before_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let spec = match (&profile.runtime, &profile.provider) {
        (ExecutionRuntime::Plugin, ExecutionProvider::ClaudeCode) => {
            super::claude_code::process_spec(program, program_source, &full_prompt)
        }
        (ExecutionRuntime::Plugin, ExecutionProvider::Codex) => {
            super::codex::process_spec(program, program_source, &request.project_path, &full_prompt)
        }
        (ExecutionRuntime::Plugin, ExecutionProvider::KimiCli) => {
            super::kimi_cli::process_spec(program, program_source, &full_prompt)
        }
        (ExecutionRuntime::Plugin, ExecutionProvider::GrokBuild) => super::grok_cli::process_spec(
            program,
            program_source,
            &request.project_path,
            &full_prompt,
        ),
        (ExecutionRuntime::BuiltIn, ExecutionProvider::GrokBuild) => unreachable!(),
        _ => {
            return Err(EngineError::InvalidConfiguration(
                "执行模式与引擎组合无效".to_string(),
            ))
        }
    };
    let display_name = spec.display_name;
    let output = super::process_runner::run_process(
        spec,
        &request.project_path,
        &request.execution_id,
        state,
    )
    .await?;
    let after_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let file_changes =
        crate::test_runner::detect_changes(&before_files, &after_files, &request.project_path);
    let engine_failure_kind = (!output.success)
        .then(|| super::classify_process_failure(output.exit_code, &output.stdout, &output.stderr));
    let error_log = if output.success {
        String::new()
    } else {
        format!(
            "{} 执行失败 (exit code: {:?}, kind: {:?})\nstdout:\n{}\nstderr:\n{}",
            display_name, output.exit_code, engine_failure_kind, output.stdout, output.stderr
        )
    };
    let combined_output = format!(
        "=== 执行日志 ===\n执行引擎：{}\n小阶段 ID：{}\n\n=== 提示词 ===\n{}\n\n=== stdout ===\n{}\n=== stderr ===\n{}",
        display_name, request.subtask_id, full_prompt, output.stdout, output.stderr
    );
    Ok(ExecutionResult {
        success: output.success,
        output: combined_output,
        error_log,
        file_changes,
        exit_code: output.exit_code,
        engine_provider: Some(profile.provider.clone()),
        engine_runtime: profile.runtime.clone(),
        engine_settings_revision: prepared.operation.settings.revision,
        engine_source_revision: String::new(),
        engine_api_backend: String::new(),
        stdout: output.stdout,
        stderr: output.stderr,
        engine_failure_kind,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_profiles() {
        for provider in [
            ExecutionProvider::ClaudeCode,
            ExecutionProvider::Codex,
            ExecutionProvider::KimiCli,
            ExecutionProvider::GrokBuild,
        ] {
            assert!(validate_profile(&ExecutionProfile {
                runtime: ExecutionRuntime::Plugin,
                provider,
                permission_profile: PermissionProfile::Unattended,
                profile_revision: 2,
            })
            .is_ok());
        }
        assert!(validate_profile(&ExecutionProfile {
            runtime: ExecutionRuntime::BuiltIn,
            provider: ExecutionProvider::GrokBuild,
            permission_profile: PermissionProfile::Unattended,
            profile_revision: 1,
        })
        .is_ok());
    }

    #[test]
    fn rejects_invalid_combinations_and_interactive_mode() {
        let mut profile = ExecutionProfile {
            runtime: ExecutionRuntime::BuiltIn,
            provider: ExecutionProvider::ClaudeCode,
            ..ExecutionProfile::default()
        };
        assert!(validate_profile(&profile).is_err());
        profile.runtime = ExecutionRuntime::Plugin;
        profile.permission_profile = PermissionProfile::Interactive;
        assert!(validate_profile(&profile).is_err());
    }
}
