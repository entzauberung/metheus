use crate::project::{Milestone, Project, Subtask, SubtaskStatus};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const MAX_TASK_TREE_DEPTH: u32 = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskNodeAddress {
    pub milestone_id: String,
    pub mid_stage_id: String,
    pub top_level_task_id: String,
    pub task_id: String,
    #[serde(default)]
    pub ancestor_task_ids: Vec<String>,
    pub depth: u32,
    pub dependencies_satisfied: bool,
}

impl TaskNodeAddress {
    pub fn task_path(&self) -> Vec<String> {
        self.ancestor_task_ids
            .iter()
            .cloned()
            .chain(std::iter::once(self.task_id.clone()))
            .collect()
    }
}

pub fn is_terminal(status: &SubtaskStatus) -> bool {
    matches!(
        status,
        SubtaskStatus::Passed | SubtaskStatus::AcceptedDeviation | SubtaskStatus::Skipped
    )
}

pub fn validate_project_tree(project: &Project) -> Result<(), String> {
    let mut tasks = BTreeMap::<String, Vec<String>>::new();
    for milestone in &project.milestones {
        collect_tasks(&milestone.subtasks, 0, &mut tasks)?;
        for stage in &milestone.mid_stages {
            collect_tasks(&stage.subtasks, 0, &mut tasks)?;
        }
    }

    for (task_id, dependencies) in &tasks {
        for dependency in dependencies {
            if !tasks.contains_key(dependency) {
                return Err(format!(
                    "任务 {} 引用了不存在的依赖任务：{}",
                    task_id, dependency
                ));
            }
        }
    }

    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for task_id in tasks.keys() {
        validate_dependency_graph(task_id, &tasks, &mut visiting, &mut visited)?;
    }
    Ok(())
}

fn collect_tasks(
    tasks: &[Subtask],
    depth: u32,
    collected: &mut BTreeMap<String, Vec<String>>,
) -> Result<(), String> {
    if depth > MAX_TASK_TREE_DEPTH {
        return Err(format!("任务树深度超过安全上限 {}", MAX_TASK_TREE_DEPTH));
    }
    for task in tasks {
        if task.id.trim().is_empty() {
            return Err("任务树包含空任务标识".to_string());
        }
        if collected
            .insert(task.id.clone(), task.depends_on.clone())
            .is_some()
        {
            return Err(format!("任务树包含重复任务标识：{}", task.id));
        }
        collect_tasks(&task.child_tasks, depth.saturating_add(1), collected)?;
    }
    Ok(())
}

fn validate_dependency_graph(
    task_id: &str,
    tasks: &BTreeMap<String, Vec<String>>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
) -> Result<(), String> {
    if visited.contains(task_id) {
        return Ok(());
    }
    if !visiting.insert(task_id.to_string()) {
        return Err(format!("任务依赖形成循环：{}", task_id));
    }
    if let Some(dependencies) = tasks.get(task_id) {
        for dependency in dependencies {
            validate_dependency_graph(dependency, tasks, visiting, visited)?;
        }
    }
    visiting.remove(task_id);
    visited.insert(task_id.to_string());
    Ok(())
}

pub fn locate_task(project: &Project, task_id: &str) -> Result<Option<TaskNodeAddress>, String> {
    validate_project_tree(project)?;
    Ok(locate_task_unchecked(project, task_id))
}

fn locate_task_unchecked(project: &Project, task_id: &str) -> Option<TaskNodeAddress> {
    for milestone in &project.milestones {
        if let Some(address) = locate_in_roots(&milestone.subtasks, task_id, &milestone.id, "") {
            return Some(with_dependency_state(project, address));
        }
        for stage in &milestone.mid_stages {
            if let Some(address) =
                locate_in_roots(&stage.subtasks, task_id, &milestone.id, &stage.id)
            {
                return Some(with_dependency_state(project, address));
            }
        }
    }
    None
}

fn locate_in_roots(
    tasks: &[Subtask],
    task_id: &str,
    milestone_id: &str,
    mid_stage_id: &str,
) -> Option<TaskNodeAddress> {
    for task in tasks {
        let mut ancestors = Vec::new();
        if let Some((depth, top_level_task_id)) = locate_in_task(task, task_id, 0, &mut ancestors) {
            return Some(TaskNodeAddress {
                milestone_id: milestone_id.to_string(),
                mid_stage_id: mid_stage_id.to_string(),
                top_level_task_id,
                task_id: task_id.to_string(),
                ancestor_task_ids: ancestors,
                depth,
                dependencies_satisfied: false,
            });
        }
    }
    None
}

fn locate_in_task(
    task: &Subtask,
    task_id: &str,
    depth: u32,
    ancestors: &mut Vec<String>,
) -> Option<(u32, String)> {
    if task.id == task_id {
        return Some((depth, task.id.clone()));
    }
    ancestors.push(task.id.clone());
    for child in &task.child_tasks {
        if let Some((found_depth, _)) =
            locate_in_task(child, task_id, depth.saturating_add(1), ancestors)
        {
            return Some((found_depth, task.id.clone()));
        }
    }
    ancestors.pop();
    None
}

fn with_dependency_state(project: &Project, mut address: TaskNodeAddress) -> TaskNodeAddress {
    address.dependencies_satisfied = find_task_unchecked(project, &address.task_id)
        .is_some_and(|task| dependencies_satisfied(project, task));
    address
}

pub fn find_task<'a>(project: &'a Project, task_id: &str) -> Result<Option<&'a Subtask>, String> {
    validate_project_tree(project)?;
    Ok(find_task_unchecked(project, task_id))
}

fn find_task_unchecked<'a>(project: &'a Project, task_id: &str) -> Option<&'a Subtask> {
    for milestone in &project.milestones {
        if let Some(task) = find_in_roots(&milestone.subtasks, task_id) {
            return Some(task);
        }
        for stage in &milestone.mid_stages {
            if let Some(task) = find_in_roots(&stage.subtasks, task_id) {
                return Some(task);
            }
        }
    }
    None
}

pub fn find_descendant<'a>(task: &'a Subtask, task_id: &str) -> Option<&'a Subtask> {
    if task.id == task_id {
        return Some(task);
    }
    task.child_tasks
        .iter()
        .find_map(|child| find_descendant(child, task_id))
}

fn find_in_roots<'a>(tasks: &'a [Subtask], task_id: &str) -> Option<&'a Subtask> {
    tasks.iter().find_map(|task| find_descendant(task, task_id))
}

pub fn find_task_mut<'a>(
    project: &'a mut Project,
    task_id: &str,
) -> Result<Option<&'a mut Subtask>, String> {
    validate_project_tree(project)?;
    for milestone in &mut project.milestones {
        if let Some(task) = find_in_roots_mut(&mut milestone.subtasks, task_id) {
            return Ok(Some(task));
        }
        for stage in &mut milestone.mid_stages {
            if let Some(task) = find_in_roots_mut(&mut stage.subtasks, task_id) {
                return Ok(Some(task));
            }
        }
    }
    Ok(None)
}

fn find_in_roots_mut<'a>(tasks: &'a mut [Subtask], task_id: &str) -> Option<&'a mut Subtask> {
    for task in tasks {
        if task.id == task_id {
            return Some(task);
        }
        if let Some(found) = find_in_roots_mut(&mut task.child_tasks, task_id) {
            return Some(found);
        }
    }
    None
}

pub(crate) fn find_task_in_milestones_mut<'a>(
    milestones: &'a mut [Milestone],
    task_id: &str,
) -> Option<&'a mut Subtask> {
    for milestone in milestones {
        if let Some(task) = find_in_roots_mut(&mut milestone.subtasks, task_id) {
            return Some(task);
        }
        for stage in &mut milestone.mid_stages {
            if let Some(task) = find_in_roots_mut(&mut stage.subtasks, task_id) {
                return Some(task);
            }
        }
    }
    None
}

pub fn select_current_leaf(project: &Project) -> Result<Option<TaskNodeAddress>, String> {
    validate_project_tree(project)?;
    if let Some(session) = project
        .execution_session
        .as_ref()
        .filter(|session| session.active && !session.subtask_id.is_empty())
    {
        let address = locate_task_unchecked(project, &session.subtask_id)
            .ok_or_else(|| format!("活动执行会话指向不存在的任务：{}", session.subtask_id))?;
        let task = find_task_unchecked(project, &session.subtask_id)
            .ok_or_else(|| format!("活动执行会话指向不存在的任务：{}", session.subtask_id))?;
        if !task.child_tasks.is_empty() {
            return Err(format!(
                "活动执行会话指向父任务 {}，该任务已有子任务，必须人工迁移会话",
                task.id
            ));
        }
        if address.milestone_id != session.milestone_id
            || address.mid_stage_id != session.mid_stage_id
        {
            return Err("活动执行会话的任务路径与磁盘任务树不一致".to_string());
        }
        return Ok(Some(with_dependency_state(project, address)));
    }

    let Some(milestone) = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == project.current_milestone_id)
    else {
        return Ok(None);
    };
    let (tasks, mid_stage_id) = if let Some(stage) = milestone
        .mid_stages
        .iter()
        .find(|stage| stage.id == project.current_mid_stage_id)
    {
        (stage.subtasks.as_slice(), stage.id.as_str())
    } else {
        (milestone.subtasks.as_slice(), "")
    };
    for task in tasks {
        let mut ancestors = Vec::new();
        if let Some((leaf, depth, top_level_task_id)) =
            first_available_leaf(project, task, 0, &mut ancestors)
        {
            return Ok(Some(TaskNodeAddress {
                milestone_id: milestone.id.clone(),
                mid_stage_id: mid_stage_id.to_string(),
                top_level_task_id,
                task_id: leaf.id.clone(),
                ancestor_task_ids: ancestors,
                depth,
                dependencies_satisfied: true,
            }));
        }
    }
    Ok(None)
}

pub fn first_pending_task(project: &Project) -> Result<Option<TaskNodeAddress>, String> {
    select_current_leaf(project)
}

pub fn first_available_descendant_leaf(
    project: &Project,
    parent_task_id: &str,
) -> Result<Option<TaskNodeAddress>, String> {
    validate_project_tree(project)?;
    let Some(parent_address) = locate_task_unchecked(project, parent_task_id) else {
        return Ok(None);
    };
    let parent = find_task_unchecked(project, parent_task_id)
        .ok_or_else(|| format!("父任务不存在：{}", parent_task_id))?;
    let mut ancestors = parent_address.ancestor_task_ids.clone();
    let Some((leaf, depth, _)) =
        first_available_leaf(project, parent, parent_address.depth, &mut ancestors)
    else {
        return Ok(None);
    };
    Ok(Some(TaskNodeAddress {
        milestone_id: parent_address.milestone_id,
        mid_stage_id: parent_address.mid_stage_id,
        top_level_task_id: parent_address.top_level_task_id,
        task_id: leaf.id.clone(),
        ancestor_task_ids: ancestors,
        depth,
        dependencies_satisfied: true,
    }))
}

pub fn leaf_addresses_in_scope(
    project: &Project,
    milestone_id: &str,
    mid_stage_id: &str,
) -> Result<Vec<TaskNodeAddress>, String> {
    validate_project_tree(project)?;
    let milestone = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| format!("大阶段不存在：{}", milestone_id))?;
    let tasks = if mid_stage_id.is_empty() {
        milestone.subtasks.as_slice()
    } else {
        milestone
            .mid_stages
            .iter()
            .find(|stage| stage.id == mid_stage_id)
            .ok_or_else(|| format!("中阶段不存在：{}", mid_stage_id))?
            .subtasks
            .as_slice()
    };
    let mut addresses = Vec::new();
    for task in tasks {
        let mut ancestors = Vec::new();
        collect_leaf_addresses(
            project,
            task,
            milestone_id,
            mid_stage_id,
            &task.id,
            0,
            &mut ancestors,
            &mut addresses,
        );
    }
    Ok(addresses)
}

#[allow(clippy::too_many_arguments)]
fn collect_leaf_addresses(
    project: &Project,
    task: &Subtask,
    milestone_id: &str,
    mid_stage_id: &str,
    top_level_task_id: &str,
    depth: u32,
    ancestors: &mut Vec<String>,
    addresses: &mut Vec<TaskNodeAddress>,
) {
    if task.child_tasks.is_empty() {
        addresses.push(TaskNodeAddress {
            milestone_id: milestone_id.to_string(),
            mid_stage_id: mid_stage_id.to_string(),
            top_level_task_id: top_level_task_id.to_string(),
            task_id: task.id.clone(),
            ancestor_task_ids: ancestors.clone(),
            depth,
            dependencies_satisfied: dependencies_satisfied(project, task),
        });
        return;
    }
    ancestors.push(task.id.clone());
    for child in &task.child_tasks {
        collect_leaf_addresses(
            project,
            child,
            milestone_id,
            mid_stage_id,
            top_level_task_id,
            depth.saturating_add(1),
            ancestors,
            addresses,
        );
    }
    ancestors.pop();
}

fn first_available_leaf<'a>(
    project: &Project,
    task: &'a Subtask,
    depth: u32,
    ancestors: &mut Vec<String>,
) -> Option<(&'a Subtask, u32, String)> {
    if is_terminal(&task.status) || !dependencies_satisfied(project, task) {
        return None;
    }
    if task.child_tasks.is_empty() {
        return Some((task, depth, task.id.clone()));
    }
    ancestors.push(task.id.clone());
    for child in &task.child_tasks {
        if let Some((leaf, child_depth, _)) =
            first_available_leaf(project, child, depth.saturating_add(1), ancestors)
        {
            return Some((leaf, child_depth, task.id.clone()));
        }
    }
    ancestors.pop();
    None
}

fn dependencies_satisfied(project: &Project, task: &Subtask) -> bool {
    task.depends_on.iter().all(|dependency_id| {
        find_task_unchecked(project, dependency_id)
            .is_some_and(|dependency| is_terminal(&dependency.status))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Milestone, MilestoneStatus, StageMode};

    fn task(id: &str, status: SubtaskStatus) -> Subtask {
        Subtask {
            id: id.to_string(),
            title: id.to_string(),
            status,
            ..Default::default()
        }
    }

    fn project_with_tasks(tasks: Vec<Subtask>) -> Project {
        let mut project = Project::new("task-tree-test");
        project.current_milestone_id = "m".to_string();
        project.milestones.push(Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: MilestoneStatus::InProgress,
            mode: StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: tasks,
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: Vec::new(),
            expected_output: String::new(),
            acceptance_criteria: Vec::new(),
            ..Default::default()
        });
        project
    }

    #[test]
    fn selects_deepest_available_leaf_and_skips_blocked_sibling() {
        let first = task("first", SubtaskStatus::Pending);
        let mut blocked = task("blocked", SubtaskStatus::Pending);
        blocked.depends_on = vec!["missing-progress".to_string()];
        let available = task("available", SubtaskStatus::Pending);
        let mut parent = task("parent", SubtaskStatus::Pending);
        parent.child_tasks = vec![blocked, available];
        let project = project_with_tasks(vec![
            first,
            task("missing-progress", SubtaskStatus::Pending),
            parent,
        ]);
        let selected = select_current_leaf(&project).unwrap().unwrap();
        assert_eq!(selected.task_id, "first");

        let mut project = project;
        project.milestones[0].subtasks[0].status = SubtaskStatus::Passed;
        let selected = select_current_leaf(&project).unwrap().unwrap();
        assert_eq!(selected.task_id, "missing-progress");
        project.milestones[0].subtasks[1].status = SubtaskStatus::Passed;
        let selected = select_current_leaf(&project).unwrap().unwrap();
        assert_eq!(selected.task_id, "blocked");
        assert_eq!(selected.ancestor_task_ids, vec!["parent"]);
    }

    #[test]
    fn parent_with_children_is_never_selected() {
        let mut parent = task("parent", SubtaskStatus::Pending);
        parent.child_tasks = vec![task("leaf", SubtaskStatus::Pending)];
        let project = project_with_tasks(vec![parent]);
        let selected = select_current_leaf(&project).unwrap().unwrap();
        assert_eq!(selected.task_id, "leaf");
        assert_eq!(selected.top_level_task_id, "parent");
    }

    #[test]
    fn rejects_duplicate_ids_and_dependency_cycles() {
        let duplicate = project_with_tasks(vec![
            task("same", SubtaskStatus::Pending),
            task("same", SubtaskStatus::Pending),
        ]);
        assert!(validate_project_tree(&duplicate)
            .unwrap_err()
            .contains("重复"));

        let mut left = task("left", SubtaskStatus::Pending);
        left.depends_on = vec!["right".to_string()];
        let mut right = task("right", SubtaskStatus::Pending);
        right.depends_on = vec!["left".to_string()];
        let cyclic = project_with_tasks(vec![left, right]);
        assert!(validate_project_tree(&cyclic).unwrap_err().contains("循环"));
    }

    #[test]
    fn mutable_lookup_reaches_descendants() {
        let mut parent = task("parent", SubtaskStatus::Pending);
        parent.child_tasks = vec![task("leaf", SubtaskStatus::Pending)];
        let mut project = project_with_tasks(vec![parent]);
        find_task_mut(&mut project, "leaf").unwrap().unwrap().title = "updated".to_string();
        assert_eq!(
            find_task(&project, "leaf").unwrap().unwrap().title,
            "updated"
        );
    }
}
