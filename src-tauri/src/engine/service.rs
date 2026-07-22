use super::contract::{EngineError, ExecutionRequest};
use crate::pipeline::PipelineState;
use crate::project::{
    ExecutionProfile, ExecutionProvider, ExecutionResult, ExecutionRuntime, PermissionProfile,
};
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) fn validate_profile(profile: &ExecutionProfile) -> Result<(), String> {
    match (&profile.runtime, &profile.provider) {
        (ExecutionRuntime::Plugin, ExecutionProvider::ClaudeCode | ExecutionProvider::Codex) => {}
        (ExecutionRuntime::BuiltIn, ExecutionProvider::GrokBuild) => {}
        _ => return Err("执行模式与引擎组合无效".to_string()),
    }
    if profile.permission_profile != PermissionProfile::Unattended {
        return Err("当前后台流水线只支持 Unattended 权限模式".to_string());
    }
    Ok(())
}

pub(crate) async fn execute(
    profile: &ExecutionProfile,
    request: ExecutionRequest,
    state: Arc<Mutex<Option<PipelineState>>>,
) -> Result<ExecutionResult, EngineError> {
    validate_profile(profile).map_err(EngineError::PermissionError)?;
    if profile.runtime == ExecutionRuntime::BuiltIn {
        return Err(super::builtin::unavailable_error());
    }

    let full_prompt = format!(
        "{}\n\n=== V1 执行约束 ===\n允许新增、修改或删除的精确文件路径：\n- {}\n\
         1. 只执行上述任务，只能变更列出的精确文件，不得扩展到目录、相邻文件或改变架构。\n\
         2. 信息不足或发现范围外问题时，必须停止并说明阻塞原因，不得自行猜测或扩展。\n\
         3. 完成后不要输出总结，直接结束。",
        request.prompt,
        request.authorized_paths.join("\n- ")
    );
    let before_files = crate::test_runner::get_file_snapshot(&request.project_path);
    let spec = match profile.provider {
        ExecutionProvider::ClaudeCode => super::claude_code::process_spec(&full_prompt),
        ExecutionProvider::Codex => super::codex::process_spec(&request.project_path, &full_prompt),
        ExecutionProvider::GrokBuild => return Err(super::builtin::unavailable_error()),
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
    let error_log = if output.success {
        String::new()
    } else {
        format!(
            "{} 执行失败 (exit code: {:?})\nstderr:\n{}",
            display_name, output.exit_code, output.stderr
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
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_supported_profiles() {
        assert!(validate_profile(&ExecutionProfile::default()).is_ok());
        assert!(validate_profile(&ExecutionProfile {
            runtime: ExecutionRuntime::Plugin,
            provider: ExecutionProvider::Codex,
            permission_profile: PermissionProfile::Unattended,
            profile_revision: 2,
        })
        .is_ok());
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
        let mut profile = ExecutionProfile::default();
        profile.provider = ExecutionProvider::GrokBuild;
        assert!(validate_profile(&profile).is_err());
        profile.provider = ExecutionProvider::ClaudeCode;
        profile.permission_profile = PermissionProfile::Interactive;
        assert!(validate_profile(&profile).is_err());
    }
}
