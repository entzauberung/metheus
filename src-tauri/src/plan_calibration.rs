use crate::project;
use crate::AppState;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub(crate) struct PlanPatchOutput {
    pub(crate) implementation_guidance: String,
    pub(crate) context_summary: String,
    pub(crate) evidence_files: Vec<String>,
    #[serde(default)]
    pub(crate) dependency_notes: String,
    #[serde(default)]
    pub(crate) rationale: String,
}

pub(crate) fn immutable_contract(task: &project::Subtask) -> Result<String, String> {
    serde_json::to_string(&(
        &task.id,
        task.order,
        &task.goal,
        &task.allowed_file_paths,
        &task.new_file_paths,
        &task.acceptance_criteria,
        &task.stop_rules,
        &task.depends_on,
    ))
    .map_err(|error| error.to_string())
}

fn requires_ai_patch(
    previous: Option<&project::ProjectFactSnapshot>,
    current: &project::ProjectFactSnapshot,
) -> bool {
    crate::project_facts::has_drift(previous, current)
}

fn patch_context(
    task: &project::Subtask,
    current: &project::ProjectFactSnapshot,
) -> Result<String, String> {
    let task_json = serde_json::to_string_pretty(&serde_json::json!({
        "title": task.title,
        "goal": task.goal,
        "implementation_guidance": task.execution_prompt,
        "context_summary": task.context_summary,
        "evidence_files": task.evidence_files,
        "allowed_file_paths": task.allowed_file_paths,
        "new_file_paths": task.new_file_paths,
        "acceptance_criteria": task.acceptance_criteria,
        "required_identifiers": task.required_identifiers,
        "stop_rules": task.stop_rules,
        "dependency_notes": task.dependency_notes,
    }))
    .map_err(|error| error.to_string())?;
    let facts = serde_json::to_string_pretty(current).map_err(|error| error.to_string())?;
    Ok(format!(
        "当前任务：\n{task_json}\n\n最新项目事实：\n{facts}"
    ))
}

pub(crate) async fn calibrate_next_subtask(project: &mut project::Project) -> Result<bool, String> {
    let milestone_id = project.current_milestone_id.clone();
    let mid_stage_id = project.current_mid_stage_id.clone();
    let accepted_deviations = crate::project_facts::accepted_deviations(project);
    let task = project
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid| mid.id == mid_stage_id)
        })
        .and_then(|mid| {
            mid.subtasks
                .iter()
                .find(|task| task.status == project::SubtaskStatus::Pending)
        })
        .cloned()
        .ok_or_else(|| "没有待校准的小阶段。".to_string())?;
    let paths = crate::project_facts::snapshot_paths(&task);
    let current = crate::project_facts::capture_with_identifiers(
        &project.project_path,
        &paths,
        accepted_deviations,
        &task.required_identifiers,
    )?;
    if task.fact_snapshot.is_none() {
        let item = project
            .milestones
            .iter_mut()
            .find(|milestone| milestone.id == milestone_id)
            .and_then(|milestone| {
                milestone
                    .mid_stages
                    .iter_mut()
                    .find(|mid| mid.id == mid_stage_id)
            })
            .and_then(|mid| mid.subtasks.iter_mut().find(|item| item.id == task.id))
            .ok_or_else(|| "扫描期间下一任务已变化。".to_string())?;
        item.fact_snapshot = Some(current);
        project.workflow_state.data_revision =
            project.workflow_state.data_revision.saturating_add(1);
        project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
        return Ok(false);
    }
    if !requires_ai_patch(task.fact_snapshot.as_ref(), &current) {
        return Ok(false);
    }

    let contract_before = immutable_contract(&task)?;
    let reply = crate::api::call_deepseek_api_json(
        crate::prompts::PLAN_PATCH_PROMPT,
        &patch_context(&task, &current)?,
    )
    .await
    .map_err(|error| format!("下一任务滚动校准失败：{}", error))?;
    let patch: PlanPatchOutput = crate::json_utils::parse_json_with_retry(&reply)
        .await
        .map_err(|error| format!("计划补丁解析失败：{}", error))?;
    if patch.implementation_guidance.trim().is_empty()
        || patch.context_summary.trim().is_empty()
        || patch.evidence_files.is_empty()
        || patch.dependency_notes.trim().is_empty()
    {
        return Err("计划补丁缺少实现指引、背景、证据文件或依赖说明。".to_string());
    }

    let item = project
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == milestone_id)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter_mut()
                .find(|mid| mid.id == mid_stage_id)
        })
        .and_then(|mid| mid.subtasks.iter_mut().find(|item| item.id == task.id))
        .ok_or_else(|| "校准期间下一任务已变化。".to_string())?;
    item.execution_prompt = patch.implementation_guidance.trim().to_string();
    item.context_summary = patch.context_summary.trim().to_string();
    item.evidence_files = patch.evidence_files;
    item.dependency_notes = patch.dependency_notes.trim().to_string();
    item.fact_snapshot = Some(current);
    item.plan_patch_revision = item.plan_patch_revision.saturating_add(1);
    crate::plan_contract::hydrate_subtask_contract(item);
    if immutable_contract(item)? != contract_before {
        return Err("计划补丁改变了不可变任务契约，已拒绝。".to_string());
    }
    crate::plan_contract::validate_subtask(item, "滚动校准任务")?;

    crate::pipeline::write_execution_history(
        project,
        "info",
        project::ExecutionEventType::PlanCalibrationApplied,
        format!(
            "下一任务已按最新项目事实校准{}",
            if patch.rationale.trim().is_empty() {
                String::new()
            } else {
                format!("：{}", patch.rationale.trim())
            }
        ),
        Some(&milestone_id),
        Some(&mid_stage_id),
        Some(&task.id),
    );
    project.workflow_state.data_revision = project.workflow_state.data_revision.saturating_add(1);
    project.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    Ok(true)
}

#[tauri::command]
pub(crate) async fn calibrate_next_subtask_command(
    state: tauri::State<'_, AppState>,
    project_name: String,
) -> Result<project::Project, String> {
    let initial = crate::load_project(&project_name)?;
    let initial_revision = initial.workflow_state.data_revision;
    let initial_milestone = initial.current_milestone_id.clone();
    let initial_mid_stage = initial.current_mid_stage_id.clone();
    let initial_step = initial.workflow_state.current_step.clone();
    let initial_autopilot = initial
        .workflow_state
        .autopilot_state
        .as_ref()
        .map(|state| (state.active, state.run_status.clone()));
    let initial_task = initial
        .milestones
        .iter()
        .find(|milestone| milestone.id == initial_milestone)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid| mid.id == initial_mid_stage)
        })
        .and_then(|mid| {
            mid.subtasks
                .iter()
                .find(|task| task.status == project::SubtaskStatus::Pending)
        })
        .ok_or_else(|| "没有待校准的小阶段。".to_string())?;
    let initial_task_id = initial_task.id.clone();
    let initial_facts = crate::project_facts::capture_with_identifiers(
        &initial.project_path,
        &crate::project_facts::snapshot_paths(initial_task),
        crate::project_facts::accepted_deviations(&initial),
        &initial_task.required_identifiers,
    )?;
    let mut candidate = initial.clone();
    let changed = calibrate_next_subtask(&mut candidate).await?;
    let candidate_mutated = candidate.workflow_state.data_revision != initial_revision;

    let _pipeline_guard = state.pipeline_state.lock().await;
    let latest = crate::load_project(&project_name)?;
    if latest.workflow_state.data_revision != initial_revision
        || latest.current_milestone_id != initial_milestone
        || latest.current_mid_stage_id != initial_mid_stage
        || latest.workflow_state.current_step != initial_step
        || latest
            .workflow_state
            .autopilot_state
            .as_ref()
            .map(|state| (state.active, state.run_status.clone()))
            != initial_autopilot
    {
        return Err("滚动校准期间项目状态已变化，已丢弃旧计划补丁。".to_string());
    }

    let latest_task = latest
        .milestones
        .iter()
        .find(|milestone| milestone.id == initial_milestone)
        .and_then(|milestone| {
            milestone
                .mid_stages
                .iter()
                .find(|mid| mid.id == initial_mid_stage)
        })
        .and_then(|mid| {
            mid.subtasks
                .iter()
                .find(|task| task.status == project::SubtaskStatus::Pending)
        })
        .ok_or_else(|| "滚动校准提交时待处理任务已变化。".to_string())?;
    if latest_task.id != initial_task_id {
        return Err("滚动校准期间下一任务已变化，已丢弃旧计划补丁。".to_string());
    }
    let latest_facts = crate::project_facts::capture_with_identifiers(
        &latest.project_path,
        &crate::project_facts::snapshot_paths(latest_task),
        crate::project_facts::accepted_deviations(&latest),
        &latest_task.required_identifiers,
    )?;
    if latest_facts.git_head != initial_facts.git_head
        || latest_facts.structural_fingerprint != initial_facts.structural_fingerprint
    {
        return Err("滚动校准期间 Git 或项目事实已变化，已丢弃旧计划补丁。".to_string());
    }
    if !candidate_mutated {
        return crate::save_and_reload_project(&latest);
    }
    debug_assert!(changed || initial_task.fact_snapshot.is_none());
    crate::save_and_reload_project(&candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_contract_ignores_patchable_fields() {
        let mut task = project::Subtask {
            id: "1".to_string(),
            goal: "goal".to_string(),
            allowed_file_paths: vec!["a.ts".to_string()],
            acceptance_criteria: vec!["works".to_string()],
            ..Default::default()
        };
        let before = immutable_contract(&task).unwrap();
        task.execution_prompt = "new guidance".to_string();
        task.context_summary = "new facts".to_string();
        assert_eq!(before, immutable_contract(&task).unwrap());
    }

    #[test]
    fn ai_patch_is_requested_only_for_fact_drift() {
        let previous = project::ProjectFactSnapshot {
            structural_fingerprint: "stable".to_string(),
            ..Default::default()
        };
        assert!(!requires_ai_patch(Some(&previous), &previous));
        let current = project::ProjectFactSnapshot {
            structural_fingerprint: "changed".to_string(),
            ..Default::default()
        };
        assert!(requires_ai_patch(Some(&previous), &current));
    }
}
