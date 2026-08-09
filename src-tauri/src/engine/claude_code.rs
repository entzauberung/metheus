use super::contract::{OutputProtocol, ProcessSpec, ProgramSource};
use crate::project::EngineFailureKind;
use std::ffi::OsString;
use std::path::Path;

pub(super) fn process_spec(
    program: OsString,
    program_source: ProgramSource,
    prompt: &str,
) -> ProcessSpec {
    ProcessSpec {
        display_name: "Claude Code",
        program,
        args: vec![
            OsString::from("--dangerously-skip-permissions"),
            OsString::from("-p"),
            OsString::from(prompt),
        ],
        stdin_payload: None,
        environment: vec![],
        environment_remove: vec![],
        output_protocol: OutputProtocol::RawText,
        program_source,
        timeout_secs: crate::constants::EXECUTION_ENGINE_TIMEOUT_SECS,
    }
}

pub(super) async fn capability_probe(program: &Path) -> Result<Vec<String>, String> {
    let output = super::health::command_output(program, &["--help"])
        .await
        .ok_or_else(|| "Claude Code 能力探测超时或启动失败".to_string())?;
    if !output.status.success() {
        return Err("Claude Code 无法输出帮助信息".to_string());
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for flag in ["--dangerously-skip-permissions", "-p"] {
        if !help.contains(flag) {
            return Err(format!("当前 Claude Code 不支持必需能力 {flag}"));
        }
    }
    Ok(vec![
        "unattended".to_string(),
        "non-interactive".to_string(),
    ])
}

pub(super) async fn online_auth_probe(
    program: &Path,
    empty_directory: &Path,
) -> Result<super::contract::EngineAuthVerificationMethod, EngineFailureKind> {
    let spec = process_spec(
        program.as_os_str().to_owned(),
        ProgramSource::SettingsOverride,
        super::health::MINIMAL_PROBE_PROMPT,
    );
    super::health::run_minimal_process_probe(spec, empty_directory).await?;
    Ok(super::contract::EngineAuthVerificationMethod::OnlineMinimalRequest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_noninteractive_unattended_command() {
        let spec = process_spec(
            OsString::from("claude"),
            ProgramSource::PathSearch,
            "approved prompt",
        );
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect();
        assert_eq!(spec.program, OsString::from("claude"));
        assert_eq!(
            args,
            ["--dangerously-skip-permissions", "-p", "approved prompt"]
        );
        assert!(!args.iter().any(|argument| argument == "--model"));
        assert!(spec.stdin_payload.is_none());
        assert_eq!(spec.output_protocol, OutputProtocol::RawText);
        assert!(spec.environment.is_empty());
        assert!(spec.environment_remove.is_empty());
    }
}
