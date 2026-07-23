use crate::project;
use std::collections::BTreeSet;

const MAX_RECORDS: usize = 50;
const MAX_MATCHES: usize = 3;

fn intersects(left: &[String], right: &[String]) -> bool {
    let values = left.iter().map(String::as_str).collect::<BTreeSet<_>>();
    right.iter().any(|value| values.contains(value.as_str()))
}

pub(crate) fn matching<'a>(
    project: &'a project::Project,
    subtask: &project::Subtask,
    failure_domain: Option<&str>,
) -> Vec<&'a project::RecoveryLearningRecord> {
    project
        .recovery_learning
        .iter()
        .rev()
        .filter(|record| {
            failure_domain.is_none_or(|domain| record.failure_domain == domain)
                && (intersects(&record.related_paths, &subtask.allowed_file_paths)
                    || intersects(&record.required_identifiers, &subtask.required_identifiers))
        })
        .take(MAX_MATCHES)
        .collect()
}

pub(crate) fn render_matching(
    project: &project::Project,
    subtask: &project::Subtask,
    failure_domain: Option<&str>,
) -> String {
    let records = matching(project, subtask, failure_domain);
    if records.is_empty() {
        return String::new();
    }
    records
        .into_iter()
        .map(|record| {
            format!(
                "- [{}] 策略={}；结果={}；稳定约束={}",
                record.failure_domain,
                record.strategy,
                if record.succeeded {
                    "成功"
                } else {
                    "失败，避免重复"
                },
                if record.stable_constraint.is_empty() {
                    "无"
                } else {
                    &record.stable_constraint
                },
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn record(
    project: &mut project::Project,
    recovery: &project::RecoveryState,
    subtask: &project::Subtask,
    strategy: &str,
    succeeded: bool,
    stable_constraint: &str,
) {
    let entry = project::RecoveryLearningRecord {
        failure_signature: recovery.error_signature.clone(),
        failure_domain: format!("{:?}", recovery.error_kind),
        strategy: strategy.trim().to_string(),
        succeeded,
        related_paths: subtask.allowed_file_paths.clone(),
        required_identifiers: subtask.required_identifiers.clone(),
        stable_constraint: stable_constraint.trim().to_string(),
        recorded_at: chrono::Utc::now().to_rfc3339(),
    };
    project.recovery_learning.retain(|current| {
        current.failure_signature != entry.failure_signature
            || current.strategy != entry.strategy
            || current.succeeded != entry.succeeded
    });
    project.recovery_learning.push(entry);
    if project.recovery_learning.len() > MAX_RECORDS {
        let excess = project.recovery_learning.len() - MAX_RECORDS;
        project.recovery_learning.drain(0..excess);
    }
}

pub(crate) fn record_human_constraint(
    project: &mut project::Project,
    subtask: &project::Subtask,
    strategy: &str,
    constraint: &str,
) {
    let recovery = project::RecoveryState {
        error_kind: project::RecoveryErrorKind::HumanRequired,
        error_signature: format!("human:{}", subtask.id),
        ..Default::default()
    };
    record(project, &recovery, subtask, strategy, true, constraint);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_matching_recent_records_are_rendered() {
        let mut project = project::Project::new("learning");
        let task = project::Subtask {
            id: "task".to_string(),
            allowed_file_paths: vec!["index.html".to_string()],
            required_identifiers: vec!["event.preventDefault".to_string()],
            ..Default::default()
        };
        let recovery = project::RecoveryState {
            error_kind: project::RecoveryErrorKind::ReviewFailure,
            error_signature: "review:drag".to_string(),
            ..Default::default()
        };
        record(
            &mut project,
            &recovery,
            &task,
            "bind once",
            true,
            "keep event name",
        );
        let unrelated = project::Subtask {
            allowed_file_paths: vec!["other.rs".to_string()],
            ..Default::default()
        };
        assert!(render_matching(&project, &task, Some("ReviewFailure")).contains("bind once"));
        assert!(render_matching(&project, &unrelated, Some("ReviewFailure")).is_empty());
    }
}
