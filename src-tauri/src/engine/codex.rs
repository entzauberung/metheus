use super::contract::ProcessSpec;
use std::ffi::OsString;

pub(super) fn process_spec(project_path: &str, prompt: &str) -> ProcessSpec {
    ProcessSpec {
        display_name: "Codex",
        program: OsString::from("codex"),
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_noninteractive_unattended_command() {
        let spec = process_spec("/tmp/project", "approved prompt");
        let args: Vec<String> = spec
            .args
            .iter()
            .map(|argument| argument.to_string_lossy().to_string())
            .collect();
        assert_eq!(spec.program, OsString::from("codex"));
        assert_eq!(args.first().map(String::as_str), Some("exec"));
        assert!(args.windows(2).any(|pair| pair == ["-C", "/tmp/project"]));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--sandbox", "danger-full-access"]));
        assert!(args.contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
        assert_eq!(args.last().map(String::as_str), Some("-"));
        assert_eq!(spec.stdin_payload.as_deref(), Some("approved prompt"));
    }
}
