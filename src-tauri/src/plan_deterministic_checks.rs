use crate::project;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PlanDeterministicIssues {
    pub(crate) omissions: Vec<String>,
    pub(crate) out_of_scope: Vec<String>,
    pub(crate) not_executable: Vec<String>,
}

impl PlanDeterministicIssues {
    pub(crate) fn is_empty(&self) -> bool {
        self.omissions.is_empty() && self.out_of_scope.is_empty() && self.not_executable.is_empty()
    }
}

fn normalized_goal(goal: &str) -> String {
    goal.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn has_blank(values: &[String]) -> bool {
    values.iter().any(|value| value.trim().is_empty())
}

fn dependency_cycle(adjacency: &[Vec<usize>]) -> bool {
    fn visit(index: usize, adjacency: &[Vec<usize>], states: &mut [u8]) -> bool {
        match states[index] {
            1 => return true,
            2 => return false,
            _ => {}
        }
        states[index] = 1;
        if adjacency[index]
            .iter()
            .any(|dependency| visit(*dependency, adjacency, states))
        {
            return true;
        }
        states[index] = 2;
        false
    }

    let mut states = vec![0; adjacency.len()];
    (0..adjacency.len()).any(|index| visit(index, adjacency, &mut states))
}

pub(crate) fn check_execution_plan(
    subtasks: &[project::Subtask],
    max_subtasks: u32,
) -> PlanDeterministicIssues {
    let mut issues = PlanDeterministicIssues::default();
    if subtasks.is_empty() {
        issues.omissions.push("执行计划为空".to_string());
        return issues;
    }
    if subtasks.len() > max_subtasks as usize {
        issues.not_executable.push(format!(
            "小阶段数量超出工作负载画像上限：实际 {}，上限 {}",
            subtasks.len(),
            max_subtasks
        ));
    }

    let mut indexes_by_id = BTreeMap::new();
    let mut seen_ids = BTreeSet::new();
    let mut seen_orders = BTreeSet::new();
    let mut goals = BTreeMap::<String, usize>::new();

    for (index, task) in subtasks.iter().enumerate() {
        let task_number = index + 1;
        let entity = format!("第 {} 个小阶段", task_number);
        if task.id.trim().is_empty() {
            issues.not_executable.push(format!("{}缺少任务 ID", entity));
        } else {
            if !seen_ids.insert(task.id.as_str()) {
                issues
                    .not_executable
                    .push(format!("{}使用了重复任务 ID：{}", entity, task.id));
            }
            indexes_by_id.entry(task.id.as_str()).or_insert(index);
        }
        if task.order == 0 || !seen_orders.insert(task.order) {
            issues.not_executable.push(format!(
                "{}的执行顺序必须是非零且唯一的整数，实际为 {}",
                entity, task.order
            ));
        }
        if task.goal.trim().is_empty() {
            issues.omissions.push(format!("{}缺少明确目标", entity));
        } else {
            let goal = normalized_goal(&task.goal);
            if let Some(previous) = goals.insert(goal, task_number) {
                issues.not_executable.push(format!(
                    "第 {} 与第 {} 个小阶段目标明显重复：{}",
                    previous, task_number, task.goal
                ));
            }
        }
        if task.acceptance_criteria.is_empty() || has_blank(&task.acceptance_criteria) {
            issues.omissions.push(format!("{}缺少非空验收标准", entity));
        }
        if task.stop_rules.is_empty() || has_blank(&task.stop_rules) {
            issues.omissions.push(format!("{}缺少非空停止规则", entity));
        }
        if let Err(error) = crate::plan_contract::validate_subtask(task, &entity) {
            issues.out_of_scope.push(error);
        }
        if let Err(error) = crate::plan_contract::validate_execution_prompt(task, &entity) {
            issues.not_executable.push(error);
        }
    }

    let mut adjacency = vec![Vec::new(); subtasks.len()];
    for (index, task) in subtasks.iter().enumerate() {
        let entity = format!("第 {} 个小阶段", index + 1);
        for dependency_id in &task.depends_on {
            let Some(dependency_index) = indexes_by_id.get(dependency_id.as_str()).copied() else {
                issues.not_executable.push(format!(
                    "{}引用了不存在的依赖任务：{}",
                    entity, dependency_id
                ));
                continue;
            };
            adjacency[index].push(dependency_index);
            if subtasks[dependency_index].order >= task.order {
                issues.not_executable.push(format!(
                    "{}的依赖任务必须拥有更早顺序：{}",
                    entity, dependency_id
                ));
            }
        }
    }
    if dependency_cycle(&adjacency) {
        issues
            .not_executable
            .push("执行计划依赖关系存在环".to_string());
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, order: u32, goal: &str) -> project::Subtask {
        project::Subtask {
            id: id.to_string(),
            order,
            title: goal.to_string(),
            goal: goal.to_string(),
            allowed_file_paths: vec![format!("src/{id}.rs")],
            acceptance_criteria: vec!["结果可验证".to_string()],
            stop_rules: vec!["不得修改范围外文件".to_string()],
            execution_prompt: "完成目标并遵守文件范围".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn check_convergence_complete_plan_passes_deterministic_checks() {
        let first = task("one", 1, "建立配置读取");
        let mut second = task("two", 2, "接入配置界面");
        second.depends_on = vec![first.id.clone()];
        assert!(check_execution_plan(&[first, second], 6).is_empty());
    }

    #[test]
    fn check_convergence_structural_issues_are_blocking_before_ai_review() {
        let mut first = task("one", 2, "重复目标");
        first.allowed_file_paths = vec!["../outside.rs".to_string()];
        first.stop_rules.clear();
        first.depends_on = vec!["two".to_string()];
        let mut second = task("two", 1, "重复目标");
        second.acceptance_criteria.clear();
        second.depends_on = vec!["one".to_string()];

        let issues = check_execution_plan(&[first, second], 6);
        assert!(!issues.omissions.is_empty());
        assert!(!issues.out_of_scope.is_empty());
        assert!(issues
            .not_executable
            .iter()
            .any(|issue| issue.contains("目标明显重复")));
        assert!(issues
            .not_executable
            .iter()
            .any(|issue| issue.contains("存在环")));
    }

    #[test]
    fn task_count_over_profile_limit_is_blocking() {
        let issues = check_execution_plan(&[task("one", 1, "one"), task("two", 2, "two")], 1);
        assert!(issues
            .not_executable
            .iter()
            .any(|issue| issue.contains("实际 2，上限 1")));
    }
}
