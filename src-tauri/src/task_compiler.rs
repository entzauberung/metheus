use crate::project::Subtask;
use crate::task_complexity::{complexity_score, MAX_DEFAULT_SPLIT_DEPTH};
use crate::task_contract::{compile_subtask, TaskComplexity, TaskContract};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_SPLIT_LEAVES: usize = 4;
pub const MIN_INDEPENDENT_ARTIFACT_GROUPS: usize = 2;
pub const CHILD_TARGET_SIMILARITY_THRESHOLD: f32 = 0.5;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskCompileDecisionKind {
    DirectExecute,
    SplitFurther,
    HumanBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskCompileDecision {
    pub kind: TaskCompileDecisionKind,
    pub reason: String,
    pub max_depth: u32,
    pub child_count_hint: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskSplitGroup {
    pub criterion_indexes: Vec<u32>,
    pub criteria: Vec<String>,
    pub expected_artifacts: Vec<String>,
    pub related_symbols: Vec<String>,
    pub read_file_paths: Vec<String>,
    pub write_file_paths: Vec<String>,
    pub split_basis: String,
    pub independently_verifiable: bool,
    pub future_parallel_safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskSplitPlan {
    pub groups: Vec<TaskSplitGroup>,
    pub reason: String,
    pub parent_complexity: u32,
    pub maximum_child_complexity: u32,
    pub estimated_complexity_reduction: u32,
    pub safe: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskCompileResult {
    pub contract: TaskContract,
    pub complexity_score: u32,
    pub atomic: bool,
    pub decision: TaskCompileDecision,
    pub split_plan: Option<TaskSplitPlan>,
}

pub fn compile(subtask: &Subtask, parent_task_id: Option<&str>, depth: u32) -> TaskCompileResult {
    let mut contract = compile_subtask(subtask, parent_task_id, depth);
    let score = complexity_score(subtask);
    let atomic = is_atomic(subtask, contract.complexity);
    let split_plan = (!atomic).then(|| build_split_plan(subtask)).flatten();
    if let Some(plan) = split_plan.as_ref() {
        contract.estimated_complexity_reduction = plan.estimated_complexity_reduction;
        crate::task_contract::refresh_fingerprint(&mut contract);
    }
    let decision = if depth >= MAX_DEFAULT_SPLIT_DEPTH {
        TaskCompileDecision {
            kind: TaskCompileDecisionKind::HumanBoundary,
            reason: "已达到默认拆分深度，需要人工确认叶子任务边界".to_string(),
            max_depth: depth,
            child_count_hint: 0,
        }
    } else if !atomic {
        match split_plan.as_ref() {
            Some(plan) if plan.safe => TaskCompileDecision {
                kind: TaskCompileDecisionKind::SplitFurther,
                reason: plan.reason.clone(),
                max_depth: MAX_DEFAULT_SPLIT_DEPTH,
                child_count_hint: plan.groups.len() as u32,
            },
            Some(plan) if plan.groups.len() > MAX_SPLIT_LEAVES => TaskCompileDecision {
                kind: TaskCompileDecisionKind::HumanBoundary,
                reason: plan.reason.clone(),
                max_depth: MAX_DEFAULT_SPLIT_DEPTH,
                child_count_hint: 0,
            },
            _ => TaskCompileDecision {
                kind: TaskCompileDecisionKind::DirectExecute,
                reason: "无法确定多个独立产物边界，保守保持单一执行单元".to_string(),
                max_depth: MAX_DEFAULT_SPLIT_DEPTH,
                child_count_hint: 0,
            },
        }
    } else {
        TaskCompileDecision {
            kind: TaskCompileDecisionKind::DirectExecute,
            reason: "任务范围、目标和验收项可独立执行与验证".to_string(),
            max_depth: MAX_DEFAULT_SPLIT_DEPTH,
            child_count_hint: 0,
        }
    };
    TaskCompileResult {
        contract,
        complexity_score: score,
        atomic,
        decision,
        split_plan,
    }
}

pub fn build_split_plan(subtask: &Subtask) -> Option<TaskSplitPlan> {
    if subtask.acceptance_criteria.len() < 2 {
        return None;
    }
    let authorized_files = subtask
        .allowed_file_paths
        .iter()
        .chain(subtask.new_file_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    if authorized_files.len() < MIN_INDEPENDENT_ARTIFACT_GROUPS {
        return None;
    }
    let mut working = Vec::<WorkingGroup>::new();
    for (index, criterion) in subtask.acceptance_criteria.iter().enumerate() {
        let anchors = criterion_anchors(criterion, &authorized_files);
        let matching = working
            .iter()
            .enumerate()
            .filter_map(|(group_index, group)| {
                anchors_overlap(&group.anchors, &anchors).then_some(group_index)
            })
            .collect::<Vec<_>>();
        if anchors.is_empty() {
            // An unanchored acceptance item is context for the immediately preceding artifact,
            // never a split dimension of its own. Leading unanchored items remain conservative
            // and make the eventual plan unsafe instead of being guessed onto an artifact.
            if let Some(group) = working.last_mut().filter(|group| !group.anchors.is_empty()) {
                group.push(index as u32 + 1, criterion.clone(), anchors);
            } else if let Some(group) = working.iter_mut().find(|group| group.anchors.is_empty()) {
                group.push(index as u32 + 1, criterion.clone(), anchors);
            } else {
                working.push(WorkingGroup::new(
                    index as u32 + 1,
                    criterion.clone(),
                    anchors,
                ));
            }
        } else if let Some(first) = matching.first().copied() {
            working[first].push(index as u32 + 1, criterion.clone(), anchors);
            for merge_index in matching.into_iter().skip(1).rev() {
                let merged = working.remove(merge_index);
                working[first].merge(merged);
            }
        } else {
            working.push(WorkingGroup::new(
                index as u32 + 1,
                criterion.clone(),
                anchors,
            ));
        }
    }

    if working.len() < MIN_INDEPENDENT_ARTIFACT_GROUPS {
        return None;
    }
    let inferred_parent_anchors = working
        .iter()
        .flat_map(|group| group.anchors.iter())
        .collect::<BTreeSet<_>>()
        .len() as u32;
    let parent_complexity =
        complexity_score(subtask).saturating_add(inferred_parent_anchors.saturating_mul(2));
    let mut groups = working
        .into_iter()
        .map(|group| group.finish(subtask, &authorized_files))
        .collect::<Vec<_>>();
    let maximum_child_complexity = groups
        .iter()
        .map(estimate_group_complexity)
        .max()
        .unwrap_or(parent_complexity);
    let reduction = parent_complexity.saturating_sub(maximum_child_complexity);
    let within_child_limit = groups.len() <= MAX_SPLIT_LEAVES;
    let safe = within_child_limit
        && groups.iter().all(|group| group.independently_verifiable)
        && reduction > 0
        && groups.len() >= MIN_INDEPENDENT_ARTIFACT_GROUPS;
    let parallel_safe = groups_are_disjoint(&groups);
    for group in &mut groups {
        group.future_parallel_safe = parallel_safe;
    }
    let group_count = groups.len();
    let reason = if group_count > MAX_SPLIT_LEAVES {
        format!(
            "候选独立产物共 {} 个，超过单次拆分上限 {}，需要人工或重新规划",
            group_count, MAX_SPLIT_LEAVES
        )
    } else if safe {
        "任务已按独立授权文件产物拆分，子任务可分别执行与验收".to_string()
    } else {
        "候选子任务不能证明独立验收，保留人工边界".to_string()
    };
    Some(TaskSplitPlan {
        groups,
        reason,
        parent_complexity,
        maximum_child_complexity,
        estimated_complexity_reduction: reduction,
        safe,
    })
}

pub fn materialize_child_tasks(
    parent: &Subtask,
    parent_depth: u32,
    plan: &TaskSplitPlan,
) -> Result<Vec<Subtask>, String> {
    if plan.groups.len() > MAX_SPLIT_LEAVES {
        return Err(format!(
            "拆分计划包含 {} 个叶子，超过单次拆分上限 {}",
            plan.groups.len(),
            MAX_SPLIT_LEAVES
        ));
    }
    if !plan.safe || plan.groups.len() < MIN_INDEPENDENT_ARTIFACT_GROUPS {
        return Err("拆分计划未达到安全执行标准".to_string());
    }
    if plan.groups.iter().any(|group| {
        !group.independently_verifiable
            || group.expected_artifacts.is_empty()
            || group.write_file_paths.is_empty()
    }) {
        return Err("拆分计划包含无法独立验收的产物组".to_string());
    }
    let mut children = Vec::with_capacity(plan.groups.len());
    for (index, group) in plan.groups.iter().enumerate() {
        let child_id = format!("{}-child-{}", parent.id, index + 1);
        let mentioned_allowed = parent
            .allowed_file_paths
            .iter()
            .filter(|path| group.write_file_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        let mentioned_new = parent
            .new_file_paths
            .iter()
            .filter(|path| group.write_file_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        let mut child = Subtask {
            id: child_id,
            title: child_title(parent, group, index),
            prompt: format!(
                "{}\n\n当前执行单元只负责：{}",
                parent.prompt,
                group.criteria.join("；")
            ),
            status: crate::project::SubtaskStatus::Pending,
            order: index as u32 + 1,
            goal: group.criteria.join("；"),
            allowed_file_paths: if mentioned_allowed.is_empty() {
                parent.allowed_file_paths.clone()
            } else {
                mentioned_allowed
            },
            new_file_paths: mentioned_new,
            evidence_files: parent
                .evidence_files
                .iter()
                .filter(|path| {
                    group.read_file_paths.is_empty() || group.read_file_paths.contains(path)
                })
                .cloned()
                .collect(),
            context_summary: parent.context_summary.clone(),
            acceptance_criteria: group.criteria.clone(),
            stop_rules: parent.stop_rules.clone(),
            execution_prompt: format!(
                "{}\n\n当前子任务合同边界：{}",
                parent.execution_prompt,
                group.criteria.join("；")
            ),
            depends_on: parent.depends_on.clone(),
            dependency_notes: "继承父任务外部依赖；未创建机械的兄弟串联依赖".to_string(),
            expected_artifacts: group.expected_artifacts.clone(),
            related_symbols: group.related_symbols.clone(),
            read_file_paths: group.read_file_paths.clone(),
            write_file_paths: group.write_file_paths.clone(),
            split_basis: group.split_basis.clone(),
            independently_verifiable: group.independently_verifiable,
            future_parallel_safe: group.future_parallel_safe,
            parent_criterion_indexes: group.criterion_indexes.clone(),
            ..Default::default()
        };
        crate::plan_contract::hydrate_subtask_contract(&mut child);
        child.contract_snapshot = Some(compile_subtask(
            &child,
            Some(&parent.id),
            parent_depth.saturating_add(1),
        ));
        children.push(child);
    }
    Ok(children)
}

fn is_atomic(subtask: &Subtask, complexity: TaskComplexity) -> bool {
    let independent_goals = subtask
        .acceptance_criteria
        .iter()
        .filter(|criterion| criterion.contains([';', '；', '\n']))
        .count();
    independent_goals <= 1
        && subtask.acceptance_criteria.len() <= 3
        && subtask.allowed_file_paths.len() <= 4
        && !matches!(complexity, TaskComplexity::Large)
}

#[derive(Debug)]
struct WorkingGroup {
    criterion_indexes: Vec<u32>,
    criteria: Vec<String>,
    anchors: BTreeSet<String>,
}

impl WorkingGroup {
    fn new(index: u32, criterion: String, anchors: BTreeSet<String>) -> Self {
        Self {
            criterion_indexes: vec![index],
            criteria: vec![criterion],
            anchors,
        }
    }

    fn push(&mut self, index: u32, criterion: String, anchors: BTreeSet<String>) {
        self.criterion_indexes.push(index);
        self.criteria.push(criterion);
        self.anchors.extend(anchors);
    }

    fn merge(&mut self, other: Self) {
        self.criterion_indexes.extend(other.criterion_indexes);
        self.criteria.extend(other.criteria);
        self.anchors.extend(other.anchors);
    }

    fn finish(self, parent: &Subtask, authorized_files: &BTreeSet<String>) -> TaskSplitGroup {
        let write_file_paths = self
            .anchors
            .iter()
            .filter(|anchor| authorized_files.contains(*anchor))
            .cloned()
            .collect::<Vec<_>>();
        let related_symbols = parent
            .related_symbols
            .iter()
            .filter(|symbol| {
                self.criteria
                    .iter()
                    .any(|criterion| criterion.contains(symbol.as_str()))
            })
            .cloned()
            .collect::<Vec<_>>();
        let expected_artifacts = write_file_paths.clone();
        let split_basis = format!("独立写入产物：{}", write_file_paths.join("、"));
        let independently_verifiable = !write_file_paths.is_empty();
        TaskSplitGroup {
            criterion_indexes: self.criterion_indexes,
            criteria: self.criteria,
            expected_artifacts,
            related_symbols,
            read_file_paths: parent
                .evidence_files
                .iter()
                .filter(|path| self.anchors.contains(*path))
                .cloned()
                .collect(),
            write_file_paths,
            split_basis,
            independently_verifiable,
            future_parallel_safe: false,
        }
    }
}

fn criterion_anchors(criterion: &str, authorized_files: &BTreeSet<String>) -> BTreeSet<String> {
    authorized_files
        .iter()
        .filter(|path| criterion.contains(path.as_str()))
        .cloned()
        .collect()
}

fn anchors_overlap(left: &BTreeSet<String>, right: &BTreeSet<String>) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let overlap = left.intersection(right).count() as f32;
    let smaller = left.len().min(right.len()) as f32;
    overlap > 0.0 && overlap / smaller >= CHILD_TARGET_SIMILARITY_THRESHOLD
}

fn estimate_group_complexity(group: &TaskSplitGroup) -> u32 {
    (group.criteria.len() as u32).saturating_mul(3)
        + (group.write_file_paths.len() as u32).saturating_mul(2)
        + (group.read_file_paths.len() as u32)
        + (group.related_symbols.len() as u32).saturating_mul(2)
}

fn groups_are_disjoint(groups: &[TaskSplitGroup]) -> bool {
    for (index, left) in groups.iter().enumerate() {
        let left_files = left.write_file_paths.iter().collect::<BTreeSet<_>>();
        let left_symbols = left.related_symbols.iter().collect::<BTreeSet<_>>();
        for right in groups.iter().skip(index + 1) {
            if right
                .write_file_paths
                .iter()
                .any(|path| left_files.contains(path))
                || right
                    .related_symbols
                    .iter()
                    .any(|symbol| left_symbols.contains(symbol))
            {
                return false;
            }
        }
    }
    true
}

fn child_title(parent: &Subtask, group: &TaskSplitGroup, index: usize) -> String {
    let subject = group
        .expected_artifacts
        .first()
        .cloned()
        .unwrap_or_else(|| format!("执行单元 {}", index + 1));
    format!("{} · {}", parent.title, subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_task_uses_short_path() {
        let mut task = Subtask::default();
        task.id = "t".into();
        task.allowed_file_paths = vec!["index.html".into()];
        task.acceptance_criteria = vec!["DOM element exists".into()];
        assert_eq!(
            compile(&task, None, 0).decision.kind,
            TaskCompileDecisionKind::DirectExecute
        );
    }

    #[test]
    fn shared_implementation_criteria_are_grouped() {
        let mut task = Subtask::default();
        task.id = "t".into();
        task.allowed_file_paths = vec!["src/a.rs".into(), "src/b.rs".into()];
        task.acceptance_criteria = vec![
            "`render_panel` is defined in src/a.rs".into(),
            "`render_panel` handles the empty state".into(),
            "`load_data` is defined in src/b.rs".into(),
            "`load_data` reports errors".into(),
        ];
        let compiled = compile(&task, None, 0);
        assert_eq!(
            compiled.decision.kind,
            TaskCompileDecisionKind::SplitFurther
        );
        let plan = compiled.split_plan.unwrap();
        assert_eq!(plan.groups.len(), 2);
        assert_eq!(plan.groups[0].criteria.len(), 2);
        assert_eq!(plan.groups[1].criteria.len(), 2);
    }

    #[test]
    fn generic_complex_task_stays_direct_without_artifact_boundaries() {
        let mut task = Subtask::default();
        task.id = "t".into();
        task.allowed_file_paths = (0..6).map(|i| format!("src/{i}.rs")).collect();
        task.acceptance_criteria = (0..4).map(|i| format!("generic criterion {i}")).collect();
        assert_eq!(
            compile(&task, None, 0).decision.kind,
            TaskCompileDecisionKind::DirectExecute
        );
    }

    #[test]
    fn runtime_fix_one_file_identifiers_never_become_split_dimensions() {
        let mut task = Subtask::default();
        task.id = "clock".into();
        task.allowed_file_paths = vec!["index.html".into()];
        task.acceptance_criteria = vec![
            "`updateClock` exists".into(),
            "`Date` is used".into(),
            "`clock` is updated".into(),
            "`</body>` remains present".into(),
            "`0` is padded".into(),
        ];
        task.required_identifiers = vec![
            "updateClock".into(),
            "Date".into(),
            "clock".into(),
            "</body>".into(),
            "0".into(),
        ];

        let compiled = compile(&task, None, 0);
        assert_eq!(
            compiled.decision.kind,
            TaskCompileDecisionKind::DirectExecute
        );
        assert!(compiled.split_plan.is_none());
    }

    #[test]
    fn runtime_fix_more_than_four_artifacts_require_human_replanning() {
        let mut task = Subtask::default();
        task.id = "too-many-artifacts".into();
        task.allowed_file_paths = (0..5).map(|index| format!("src/{index}.rs")).collect();
        task.acceptance_criteria = (0..5)
            .map(|index| format!("src/{index}.rs independently passes its check"))
            .collect();

        let compiled = compile(&task, None, 0);
        assert_eq!(
            compiled.decision.kind,
            TaskCompileDecisionKind::HumanBoundary
        );
        let plan = compiled.split_plan.expect("应保留超限诊断计划");
        assert_eq!(plan.groups.len(), 5);
        assert!(!plan.safe);
        assert!(materialize_child_tasks(&task, 0, &plan).is_err());
    }

    #[test]
    fn runtime_fix_materialized_children_are_independently_verifiable() {
        let mut task = Subtask::default();
        task.id = "t".into();
        task.title = "Task".into();
        task.allowed_file_paths = vec!["src/a.rs".into(), "src/b.rs".into()];
        task.acceptance_criteria = vec![
            "`render_panel` in src/a.rs exists".into(),
            "`load_data` in src/b.rs exists".into(),
            "`persist_data` in src/b.rs exists".into(),
            "`validate_data` in src/b.rs exists".into(),
        ];
        let plan = build_split_plan(&task).unwrap();
        let children = materialize_child_tasks(&task, 0, &plan).unwrap();
        assert!((2..=MAX_SPLIT_LEAVES).contains(&children.len()));
        assert!(children.iter().all(|child| child.depends_on.is_empty()));
        assert!(children.iter().all(|child| {
            child.independently_verifiable
                && !child.expected_artifacts.is_empty()
                && !child.acceptance_criteria.is_empty()
        }));
        assert!(children
            .iter()
            .all(|child| child.acceptance_criteria.len() < task.acceptance_criteria.len()));
    }
}
