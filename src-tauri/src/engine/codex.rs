use super::contract::{OutputProtocol, ProcessSpec, ProgramSource};
use crate::project::EngineFailureKind;
use std::ffi::OsString;
use std::path::Path;

pub(super) fn process_spec(
    program: OsString,
    program_source: ProgramSource,
    project_path: &str,
    prompt: &str,
) -> ProcessSpec {
    ProcessSpec {
        display_name: "Codex",
        program,
        args: vec![
            OsString::from("exec"),
            OsString::from("--color"),
            OsString::from("never"),
            OsString::from("-C"),
            OsString::from(project_path),
            OsString::from("--sandbox"),
            OsString::from("danger-full-access"),
            OsString::from("--dangerously-bypass-approvals-and-sandbox"),
            OsString::from("-"),
        ],
        stdin_payload: Some(prompt.to_string()),
        environment: vec![],
        environment_remove: vec![],
        output_protocol: OutputProtocol::RawText,
        program_source,
        timeout_secs: crate::constants::EXECUTION_ENGINE_TIMEOUT_SECS,
    }
}

pub(super) async fn capability_probe(program: &Path) -> Result<Vec<String>, String> {
    let output = super::health::command_output(program, &["exec", "--help"])
        .await
        .ok_or_else(|| "Codex 能力探测超时或启动失败".to_string())?;
    if !output.status.success() {
        return Err("Codex 无法输出 exec 帮助信息".to_string());
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for flag in [
        "-C",
        "--sandbox",
        "--dangerously-bypass-approvals-and-sandbox",
    ] {
        if !help.contains(flag) {
            return Err(format!("当前 Codex 不支持必需能力 {flag}"));
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
        empty_directory.to_string_lossy().as_ref(),
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
            OsString::from("codex"),
            ProgramSource::PathSearch,
            "/tmp/project",
            "approved prompt",
        );
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect();
        assert_eq!(spec.program, OsString::from("codex"));
        assert_eq!(
            args,
            [
                "exec",
                "--color",
                "never",
                "-C",
                "/tmp/project",
                "--sandbox",
                "danger-full-access",
                "--dangerously-bypass-approvals-and-sandbox",
                "-",
            ]
        );
        assert!(!args.iter().any(|argument| {
            matches!(
                argument.as_str(),
                "--model" | "--provider" | "approved prompt"
            )
        }));
        assert_eq!(spec.stdin_payload.as_deref(), Some("approved prompt"));
        assert_eq!(spec.output_protocol, OutputProtocol::RawText);
        assert!(spec.environment.is_empty());
        assert!(spec.environment_remove.is_empty());
    }
}
