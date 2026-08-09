use super::contract::{
    EngineAuthVerificationMethod, EngineLocalAuthState, OutputProtocol, ProcessSpec, ProgramSource,
};
use crate::project::EngineFailureKind;
use std::ffi::OsString;
use std::path::Path;

pub(super) const EXECUTABLE_CANDIDATES: &[&str] = &["kimi"];

pub(super) fn process_spec(
    program: OsString,
    program_source: ProgramSource,
    prompt: &str,
) -> ProcessSpec {
    ProcessSpec {
        display_name: "Kimi CLI",
        program,
        args: vec![
            OsString::from("--auto"),
            OsString::from("--prompt"),
            OsString::from(prompt),
            OsString::from("--output-format"),
            OsString::from("stream-json"),
        ],
        stdin_payload: None,
        environment: vec![],
        environment_remove: vec![],
        output_protocol: OutputProtocol::JsonLines,
        program_source,
        timeout_secs: crate::constants::EXECUTION_ENGINE_TIMEOUT_SECS,
    }
}

pub(super) async fn capability_probe(program: &Path) -> Result<Vec<String>, String> {
    let output = super::health::command_output(program, &["--help"])
        .await
        .ok_or_else(|| "Kimi CLI 能力探测超时或启动失败".to_string())?;
    if !output.status.success() {
        return Err("Kimi CLI 无法输出帮助信息".to_string());
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for flag in ["--auto", "--prompt", "stream-json"] {
        if !help.contains(flag) {
            return Err(format!("当前 Kimi CLI 不支持必需能力 {flag}"));
        }
    }
    Ok(vec![
        "unattended".to_string(),
        "non-interactive".to_string(),
        "stream-json".to_string(),
    ])
}

pub(super) async fn passive_auth_probe(program: &Path) -> (EngineLocalAuthState, String) {
    let doctor = super::health::command_output(program, &["doctor", "config"]).await;
    if !doctor.is_some_and(|output| output.status.success()) {
        return (
            EngineLocalAuthState::Unknown,
            "Kimi CLI 无法确认本地配置有效".to_string(),
        );
    }
    let providers = super::health::command_output(program, &["provider", "list"]).await;
    match providers {
        Some(output)
            if output.status.success()
                && !String::from_utf8_lossy(&output.stdout).trim().is_empty() =>
        {
            (
                EngineLocalAuthState::ConfiguredEvidence,
                "Kimi CLI 已发现有效的本地 provider 配置".to_string(),
            )
        }
        Some(output) if output.status.success() => (
            EngineLocalAuthState::Missing,
            "Kimi CLI 未配置 provider".to_string(),
        ),
        _ => (
            EngineLocalAuthState::Unknown,
            "Kimi CLI 无法读取 provider 配置".to_string(),
        ),
    }
}

pub(super) async fn online_auth_probe(
    program: &Path,
    empty_directory: &Path,
) -> Result<EngineAuthVerificationMethod, EngineFailureKind> {
    let spec = process_spec(
        program.as_os_str().to_owned(),
        ProgramSource::SettingsOverride,
        super::health::MINIMAL_PROBE_PROMPT,
    );
    super::health::run_minimal_process_probe(spec, empty_directory).await?;
    Ok(EngineAuthVerificationMethod::OnlineMinimalRequest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_unattended_streaming_command() {
        let spec = process_spec(
            OsString::from("kimi"),
            ProgramSource::PathSearch,
            "approved prompt",
        );
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args,
            [
                "--auto",
                "--prompt",
                "approved prompt",
                "--output-format",
                "stream-json"
            ]
        );
        assert!(!args
            .iter()
            .any(|argument| { matches!(argument.as_str(), "--yolo" | "--model" | "--provider") }));
        assert_eq!(
            args.iter()
                .filter(|argument| argument.as_str() == "approved prompt")
                .count(),
            1
        );
        assert!(spec.stdin_payload.is_none());
        assert_eq!(spec.output_protocol, OutputProtocol::JsonLines);
        assert!(spec.environment.is_empty());
        assert!(spec.environment_remove.is_empty());
    }
}
