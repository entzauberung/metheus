use super::contract::{OutputProtocol, ProcessSpec, ProgramSource};
use std::ffi::OsString;

pub(super) fn process_spec(
    program: OsString,
    program_source: ProgramSource,
    prompt: &str,
) -> ProcessSpec {
    let configured_model = std::env::var("METHEUS_MODEL")
        .unwrap_or_else(|_| crate::constants::DEEPSEEK_WORKFLOW_MODEL.to_string());
    let model = if configured_model == crate::constants::DEEPSEEK_WORKFLOW_MODEL {
        configured_model
    } else {
        eprintln!(
            "[engine::claude_code] 模型 \"{}\" 不在白名单中，使用默认模型 \"{}\"",
            configured_model,
            crate::constants::DEEPSEEK_WORKFLOW_MODEL,
        );
        crate::constants::DEEPSEEK_WORKFLOW_MODEL.to_string()
    };

    ProcessSpec {
        display_name: "Claude Code",
        program,
        args: vec![
            OsString::from("--dangerously-skip-permissions"),
            OsString::from("--model"),
            OsString::from(model),
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
        assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
        assert!(args.contains(&"--model".to_string()));
        assert_eq!(args[args.len() - 2..], ["-p", "approved prompt"]);
        assert!(spec.stdin_payload.is_none());
    }
}
