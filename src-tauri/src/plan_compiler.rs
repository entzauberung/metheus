use crate::project;

fn list(items: &[String], empty: &str) -> String {
    if items.is_empty() {
        empty.to_string()
    } else {
        format!("- {}", items.join("\n- "))
    }
}

pub(crate) fn compile_execution_prompt(subtask: &project::Subtask) -> String {
    compile_execution_prompt_with_learning(subtask, "")
}

pub(crate) fn compile_execution_prompt_with_learning(
    subtask: &project::Subtask,
    matching_learning: &str,
) -> String {
    let implementation = if subtask.execution_prompt.trim().is_empty() {
        subtask.prompt.trim()
    } else {
        subtask.execution_prompt.trim()
    };
    let fact_context = subtask
        .fact_snapshot
        .as_ref()
        .map(|facts| {
            format!(
                "Git HEAD: {}\n结构指纹: {}\n当前符号: {}\n存储键: {}\nDOM id: {}\n事件绑定: {}\n已接受偏差: {}",
                facts.git_head,
                facts.structural_fingerprint,
                facts.symbols.join(", "),
                facts.storage_keys.join(", "),
                facts.dom_ids.join(", "),
                facts.event_bindings.join(", "),
                facts.accepted_deviations.join("；"),
            )
        })
        .unwrap_or_else(|| "（尚无项目事实快照；必须先读取证据文件）".to_string());

    format!(
        "任务目标：\n{}\n\n计划背景：\n{}\n\n实现指引：\n{}\n\n当前代码事实：\n{}\n\n匹配的纠错经验（仅限当前文件/标识符）：\n{}\n\n必须先读取的证据文件：\n{}\n\n不可变验收标准：\n{}\n\n精确标识符（由系统附加，不得替换命名）：\n{}\n\n依赖说明：\n{}\n\n停止规则：\n{}",
        if subtask.goal.trim().is_empty() { &subtask.title } else { &subtask.goal },
        if subtask.context_summary.trim().is_empty() { "（无额外背景）" } else { &subtask.context_summary },
        implementation,
        fact_context,
        if matching_learning.trim().is_empty() { "（无）" } else { matching_learning },
        list(&subtask.evidence_files, "（无）"),
        list(&subtask.acceptance_criteria, "（无）"),
        list(&subtask.required_identifiers, "（无）"),
        if subtask.dependency_notes.trim().is_empty() { "（无显式依赖说明）" } else { &subtask.dependency_notes },
        list(&subtask.stop_rules, "发现信息不足或范围外问题时停止"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiler_appends_exact_identifiers() {
        let mut task = project::Subtask {
            id: "task".to_string(),
            title: "drag".to_string(),
            prompt: "call preventDefault".to_string(),
            status: project::SubtaskStatus::Pending,
            required_identifiers: vec!["event.preventDefault".to_string()],
            acceptance_criteria: vec!["调用 event.preventDefault".to_string()],
            ..Default::default()
        };
        task.execution_prompt = "调用 preventDefault()".to_string();
        task.context_summary = "沿用现有 dragState".to_string();
        let compiled = compile_execution_prompt(&task);
        assert!(compiled.contains("event.preventDefault"));
        assert!(compiled.contains("沿用现有 dragState"));
    }
}
