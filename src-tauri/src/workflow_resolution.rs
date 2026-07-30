use crate::project;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum MidStageRoute {
    NeedsInitialGeneration,
    SelectExisting {
        mid_stage_id: String,
    },
    ResumeExisting {
        mid_stage_id: String,
        next_step: project::WorkflowStep,
    },
    ReviewMilestone,
    WaitHuman {
        reason: String,
    },
}

pub(crate) fn has_plan_execution_facts(mid_stage: &project::MidStage) -> bool {
    !mid_stage.git_tag.is_empty()
        || mid_stage.subtasks.iter().any(|subtask| {
            matches!(
                subtask.status,
                project::SubtaskStatus::Executing
                    | project::SubtaskStatus::AwaitingConfirmation
                    | project::SubtaskStatus::Passed
                    | project::SubtaskStatus::AcceptedDeviation
                    | project::SubtaskStatus::Skipped
            ) || subtask.auto_tag.as_ref().is_some_and(|tag| !tag.is_empty())
        })
}

pub(crate) fn resolve_selected_mid_stage_step(
    milestone: &project::Milestone,
    mid_stage: &project::MidStage,
) -> Result<project::WorkflowStep, String> {
    if mid_stage.status == project::MidStageStatus::Completed {
        return Ok(
            if milestone
                .mid_stages
                .iter()
                .all(|item| item.status == project::MidStageStatus::Completed)
            {
                project::WorkflowStep::MilestoneReview
            } else {
                project::WorkflowStep::MidStageSelection
            },
        );
    }

    if matches!(
        mid_stage.status,
        project::MidStageStatus::Rejected | project::MidStageStatus::RolledBack
    ) {
        return Err(format!(
            "中阶段「{}」处于 {:?} 状态，需要人工对账后才能继续。",
            mid_stage.title, mid_stage.status
        ));
    }

    let next_step = if mid_stage.status == project::MidStageStatus::InProgress
        || has_plan_execution_facts(mid_stage)
        || mid_stage.plan_approved_at.is_some()
        || mid_stage.plan_revision > 0
    {
        project::WorkflowStep::Execution
    } else if mid_stage
        .plan_check_result
        .as_ref()
        .is_some_and(|result| result.passed)
    {
        project::WorkflowStep::PlanApproving
    } else if mid_stage.plan_check_result.is_some()
        || mid_stage.plan_generated_at.is_some()
        || !mid_stage.subtasks.is_empty()
    {
        project::WorkflowStep::PlanCheck
    } else {
        project::WorkflowStep::PlanGeneration
    };
    Ok(next_step)
}

pub(crate) fn resolve_mid_stage_route(milestone: &project::Milestone) -> MidStageRoute {
    if milestone.mid_stages.is_empty() {
        return MidStageRoute::NeedsInitialGeneration;
    }

    let active = milestone
        .mid_stages
        .iter()
        .filter(|mid_stage| {
            mid_stage.status != project::MidStageStatus::Completed
                && (mid_stage.status == project::MidStageStatus::InProgress
                    || has_plan_execution_facts(mid_stage))
        })
        .collect::<Vec<_>>();

    if active.len() > 1 {
        return MidStageRoute::WaitHuman {
            reason: format!(
                "大阶段「{}」存在多个活动中阶段（{}），需要人工对账。",
                milestone.title,
                active
                    .iter()
                    .map(|mid_stage| mid_stage.title.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
        };
    }
    if let Some(mid_stage) = active.first() {
        return match resolve_selected_mid_stage_step(milestone, mid_stage) {
            Ok(next_step) => MidStageRoute::ResumeExisting {
                mid_stage_id: mid_stage.id.clone(),
                next_step,
            },
            Err(reason) => MidStageRoute::WaitHuman { reason },
        };
    }

    if milestone
        .mid_stages
        .iter()
        .all(|mid_stage| mid_stage.status == project::MidStageStatus::Completed)
    {
        return MidStageRoute::ReviewMilestone;
    }

    let mut ordered = milestone.mid_stages.iter().enumerate().collect::<Vec<_>>();
    ordered.sort_by_key(|(index, mid_stage)| (mid_stage.order.unwrap_or(i32::MAX), *index));
    for (_, mid_stage) in ordered {
        if mid_stage.status == project::MidStageStatus::Completed {
            continue;
        }
        if matches!(
            mid_stage.status,
            project::MidStageStatus::Rejected | project::MidStageStatus::RolledBack
        ) {
            return MidStageRoute::WaitHuman {
                reason: format!(
                    "中阶段「{}」处于 {:?} 状态，需要人工对账后才能继续。",
                    mid_stage.title, mid_stage.status
                ),
            };
        }
        return MidStageRoute::SelectExisting {
            mid_stage_id: mid_stage.id.clone(),
        };
    }

    MidStageRoute::WaitHuman {
        reason: format!("大阶段「{}」的中阶段状态无法解析。", milestone.title),
    }
}

pub(crate) fn apply_mid_stage_route(
    proj: &mut project::Project,
    route: &MidStageRoute,
) -> Result<bool, String> {
    let original_step = proj.workflow_state.current_step.clone();
    let original_mid_stage_id = proj.current_mid_stage_id.clone();
    match route {
        MidStageRoute::NeedsInitialGeneration => {
            proj.current_mid_stage_id.clear();
            proj.workflow_state.current_step = project::WorkflowStep::MidStageGeneration;
        }
        MidStageRoute::SelectExisting { mid_stage_id } => {
            let milestone = proj
                .milestones
                .iter()
                .find(|milestone| milestone.id == proj.current_milestone_id)
                .ok_or_else(|| "当前大阶段不存在。".to_string())?;
            let mid_stage = milestone
                .mid_stages
                .iter()
                .find(|mid_stage| mid_stage.id == *mid_stage_id)
                .ok_or_else(|| "待选择的中阶段不存在。".to_string())?;
            proj.workflow_state.current_step =
                resolve_selected_mid_stage_step(milestone, mid_stage)?;
            proj.current_mid_stage_id = mid_stage_id.clone();
        }
        MidStageRoute::ResumeExisting {
            mid_stage_id,
            next_step,
        } => {
            proj.current_mid_stage_id = mid_stage_id.clone();
            proj.workflow_state.current_step = next_step.clone();
        }
        MidStageRoute::ReviewMilestone => {
            proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
        }
        MidStageRoute::WaitHuman { reason } => return Err(reason.clone()),
    }
    Ok(original_step != proj.workflow_state.current_step
        || original_mid_stage_id != proj.current_mid_stage_id)
}

pub(crate) fn reconcile_mid_stage_route(proj: &mut project::Project) -> bool {
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneSelection
        || proj.current_milestone_id.is_empty()
    {
        return false;
    }
    let Some(milestone) = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == proj.current_milestone_id)
    else {
        return false;
    };
    let route = resolve_mid_stage_route(milestone);
    let Ok(changed) = apply_mid_stage_route(proj, &route) else {
        return false;
    };
    if changed {
        proj.workflow_state.data_revision += 1;
        proj.workflow_state.last_transition_at = chrono::Utc::now().to_rfc3339();
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mid_stage(id: &str, status: project::MidStageStatus, order: i32) -> project::MidStage {
        project::MidStage {
            id: id.to_string(),
            title: id.to_string(),
            version: format!("v0.1.{order}"),
            order: Some(order),
            status,
            subtasks: vec![],
            domain: None,
            test_log: None,
            created_at: String::new(),
            description: String::new(),
            tech_focus: String::new(),
            test_report: String::new(),
            completed_at: None,
            approved_at: None,
            git_tag: String::new(),
            plan_check_result: None,
            plan_approved_at: None,
            plan_revision: 0,
            plan_draft_revision: 0,
            plan_generated_at: None,
            plan_regeneration_count: 0,
            last_plan_failure_fingerprint: String::new(),
            last_plan_issue_count: 0,
            plan_no_progress_count: 0,
        }
    }

    fn milestone(mid_stages: Vec<project::MidStage>) -> project::Milestone {
        project::Milestone {
            id: "milestone-1".to_string(),
            version: "v0.1".to_string(),
            title: "测试大阶段".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: project::MilestoneStatus::InProgress,
            mode: project::StageMode::Professional,
            mid_stages,
            subtasks: vec![],
            qa_result: None,
            git_commit_hash: String::new(),
            decomposition_check: None,
            review_status: None,
            review_conclusion: None,
            approved_at: None,
            goal: String::new(),
            scope: String::new(),
            dependencies: vec![],
            expected_output: String::new(),
            acceptance_criteria: vec![],
        }
    }

    #[test]
    fn workflow_resolution_empty_list_needs_initial_generation() {
        assert_eq!(
            resolve_mid_stage_route(&milestone(vec![])),
            MidStageRoute::NeedsInitialGeneration
        );
    }

    #[test]
    fn workflow_resolution_selects_first_pending_stage() {
        let route = resolve_mid_stage_route(&milestone(vec![
            mid_stage("completed", project::MidStageStatus::Completed, 1),
            mid_stage("pending", project::MidStageStatus::Pending, 2),
            mid_stage("later", project::MidStageStatus::Ready, 3),
        ]));
        assert_eq!(
            route,
            MidStageRoute::SelectExisting {
                mid_stage_id: "pending".to_string()
            }
        );
    }

    #[test]
    fn workflow_resolution_resumes_in_progress_stage() {
        let route = resolve_mid_stage_route(&milestone(vec![
            mid_stage("pending", project::MidStageStatus::Pending, 1),
            mid_stage("active", project::MidStageStatus::InProgress, 2),
        ]));
        assert_eq!(
            route,
            MidStageRoute::ResumeExisting {
                mid_stage_id: "active".to_string(),
                next_step: project::WorkflowStep::Execution,
            }
        );
    }

    #[test]
    fn workflow_resolution_completed_list_enters_review() {
        assert_eq!(
            resolve_mid_stage_route(&milestone(vec![
                mid_stage("one", project::MidStageStatus::Completed, 1),
                mid_stage("two", project::MidStageStatus::Completed, 2),
            ])),
            MidStageRoute::ReviewMilestone
        );
    }
}
