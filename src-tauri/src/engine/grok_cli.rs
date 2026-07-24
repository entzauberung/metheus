use super::contract::{
    EngineAuthVerificationMethod, EngineLocalAuthState, OutputProtocol, ProcessSpec, ProgramSource,
};
use crate::project::EngineFailureKind;
use std::ffi::OsString;
use std::path::Path;

pub(super) const EXECUTABLE_CANDIDATES: &[&str] = &["grok"];

pub(super) fn process_spec(
    program: OsString,
    program_source: ProgramSource,
    project_path: &str,
    prompt: &str,
) -> ProcessSpec {
    ProcessSpec {
        display_name: "Grok Build CLI",
        program,
        args: vec![
            OsString::from("--cwd"),
            OsString::from(project_path),
            OsString::from("--single"),
            OsString::from(prompt),
            OsString::from("--always-approve"),
            OsString::from("--output-format"),
            OsString::from("streaming-json"),
            OsString::from("--no-memory"),
            OsString::from("--no-subagents"),
            OsString::from("--disable-web-search"),
            OsString::from("--verbatim"),
        ],
        stdin_payload: None,
        environment: vec![],
        environment_remove: vec![
            OsString::from(crate::constants::BUILTIN_GROK_BUILD_API_KEY_ENV),
            OsString::from(crate::constants::LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV),
            OsString::from(crate::constants::UPSTREAM_GROK_API_KEY_ENV),
        ],
        output_protocol: OutputProtocol::JsonLines,
        program_source,
        timeout_secs: crate::constants::EXECUTION_ENGINE_TIMEOUT_SECS,
    }
}

pub(super) async fn capability_probe(program: &Path) -> Result<Vec<String>, String> {
    let output = super::health::command_output(program, &["--help"])
        .await
        .ok_or_else(|| "Grok Build CLI 能力探测超时或启动失败".to_string())?;
    if !output.status.success() {
        return Err("Grok Build CLI 无法输出帮助信息".to_string());
    }
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for flag in [
        "--cwd",
        "--single",
        "--always-approve",
        "streaming-json",
        "--no-memory",
        "--no-subagents",
        "--disable-web-search",
        "--verbatim",
    ] {
        if !help.contains(flag) {
            return Err(format!("当前 Grok Build CLI 不支持必需能力 {flag}"));
        }
    }
    Ok(vec![
        "unattended".to_string(),
        "non-interactive".to_string(),
        "streaming-json".to_string(),
    ])
}

pub(super) async fn passive_auth_probe(program: &Path) -> (EngineLocalAuthState, String) {
    let Some(output) = super::health::command_output(program, &["inspect", "--json"]).await else {
        return (
            EngineLocalAuthState::Unknown,
            "Grok Build CLI 无法读取本地配置".to_string(),
        );
    };
    if !output.status.success() {
        return (
            EngineLocalAuthState::Unknown,
            "Grok Build CLI 本地配置探测失败".to_string(),
        );
    }
    let configured = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .ok()
        .and_then(|value| {
            value
                .get("configSources")?
                .get("layers")?
                .as_array()
                .cloned()
        })
        .is_some_and(|layers| !layers.is_empty());
    if configured {
        (
            EngineLocalAuthState::ConfiguredEvidence,
            "Grok Build CLI 已发现本地配置层".to_string(),
        )
    } else {
        (
            EngineLocalAuthState::Unknown,
            "Grok Build CLI 未发现可确认认证状态的本地证据".to_string(),
        )
    }
}

pub(super) async fn online_auth_probe(
    program: &Path,
    empty_directory: &Path,
) -> Result<EngineAuthVerificationMethod, EngineFailureKind> {
    let output = super::health::online_command_output(
        program,
        &["models"],
        empty_directory,
        &[
            crate::constants::BUILTIN_GROK_BUILD_API_KEY_ENV,
            crate::constants::LEGACY_BUILTIN_GROK_BUILD_API_KEY_ENV,
            crate::constants::UPSTREAM_GROK_API_KEY_ENV,
        ],
    )
    .await?;
    if output.status.success() && !output.stdout.is_empty() {
        Ok(EngineAuthVerificationMethod::OnlineModelList)
    } else {
        Err(super::classify_process_failure(
            output.status.code(),
            &String::from_utf8_lossy(&output.stdout),
            &String::from_utf8_lossy(&output.stderr),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_isolated_unattended_streaming_command() {
        let spec = process_spec(
            OsString::from("grok"),
            ProgramSource::PathSearch,
            "/tmp/project",
            "approved prompt",
        );
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect();
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--cwd", "/tmp/project"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--single", "approved prompt"]));
        assert!(args.contains(&"--always-approve".to_string()));
        assert!(args.contains(&"--no-memory".to_string()));
        assert!(args.contains(&"--no-subagents".to_string()));
        assert!(args.contains(&"--disable-web-search".to_string()));
        assert_eq!(spec.output_protocol, OutputProtocol::JsonLines);
        assert!(spec.environment.is_empty());
    }
}
