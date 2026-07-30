use crate::project::Subtask;
use crate::task_contract::{TaskBudgetSummary, TaskComplexity, TaskRiskLevel};

pub const MAX_DEFAULT_SPLIT_DEPTH: u32 = 8;

pub fn complexity_score(subtask: &Subtask) -> u32 {
    let files = (subtask.allowed_file_paths.len() + subtask.new_file_paths.len()) as u32;
    let criteria = subtask.acceptance_criteria.len() as u32;
    let evidence = subtask.evidence_files.len() as u32;
    let identifiers = subtask.required_identifiers.len() as u32;
    let dependencies = subtask.depends_on.len() as u32;
    let symbols = subtask.related_symbols.len() as u32;
    let artifacts = subtask.expected_artifacts.len() as u32;
    files.saturating_mul(2)
        + criteria.saturating_mul(3)
        + evidence
        + identifiers.saturating_mul(2)
        + dependencies.saturating_mul(2)
        + symbols.saturating_mul(2)
        + artifacts
        + u32::from(subtask.title.len() > 80)
        + u32::from(subtask.prompt.len() > 600) * 2
}

pub fn estimate_complexity(subtask: &Subtask) -> TaskComplexity {
    match complexity_score(subtask) {
        0..=8 => TaskComplexity::Small,
        9..=20 => TaskComplexity::Medium,
        _ => TaskComplexity::Large,
    }
}

pub fn estimate_risk(subtask: &Subtask, complexity: TaskComplexity) -> TaskRiskLevel {
    let text = format!("{} {}", subtask.title, subtask.goal).to_ascii_lowercase();
    if text.contains("migration") || text.contains("schema") || text.contains("security") {
        return TaskRiskLevel::High;
    }
    if subtask.allowed_file_paths.len() > 6 || subtask.depends_on.len() > 3 {
        return TaskRiskLevel::High;
    }
    match complexity {
        TaskComplexity::Large => TaskRiskLevel::Medium,
        TaskComplexity::Medium => TaskRiskLevel::Medium,
        TaskComplexity::Small => TaskRiskLevel::Low,
    }
}

pub fn estimate_budget(subtask: &Subtask, complexity: TaskComplexity) -> TaskBudgetSummary {
    let (level, calls, input, output): (&str, u32, u64, u64) = match complexity {
        TaskComplexity::Small => ("small", 1, 1_500, 800),
        TaskComplexity::Medium => ("medium", 2, 3_000, 1_600),
        TaskComplexity::Large => ("large", 3, 5_000, 2_500),
    };
    TaskBudgetSummary {
        level: level.to_string(),
        estimated_model_calls: if subtask.acceptance_criteria.is_empty() {
            calls.saturating_sub(1)
        } else {
            calls
        },
        estimated_input_tokens: input,
        estimated_output_tokens: output,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_file_task_is_small() {
        let mut task = Subtask::default();
        task.allowed_file_paths = vec!["index.html".into()];
        task.acceptance_criteria = vec!["element exists".into()];
        assert_eq!(estimate_complexity(&task), TaskComplexity::Small);
    }

    #[test]
    fn many_contracts_raise_complexity_and_budget() {
        let mut task = Subtask::default();
        task.allowed_file_paths = (0..8).map(|i| format!("src/{i}.rs")).collect();
        task.acceptance_criteria = (0..5).map(|i| format!("criterion {i}")).collect();
        assert_eq!(estimate_complexity(&task), TaskComplexity::Large);
        assert_eq!(estimate_budget(&task, TaskComplexity::Large).level, "large");
    }
}
