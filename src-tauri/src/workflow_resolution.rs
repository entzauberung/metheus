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

pub(crate) fn is_terminal_subtask(status: &project::SubtaskStatus) -> bool {
    matches!(
        status,
        project::SubtaskStatus::Passed
            | project::SubtaskStatus::AcceptedDeviation
            | project::SubtaskStatus::Skipped
    )
}

/// Validate and establish the single in-memory MilestoneReview boundary.
///
/// This function is deliberately synchronous and persistence-agnostic. Callers
/// own revision/timestamp updates and saving so one business transition cannot
/// accidentally increment the project revision twice.
pub(crate) fn apply_milestone_review_boundary(
    proj: &mut project::Project,
    milestone_id: &str,
    now: &str,
) -> Result<(), String> {
    if milestone_id.is_empty() {
        return Err("未选择大阶段。".to_string());
    }
    if proj.current_milestone_id != milestone_id {
        return Err(format!(
            "当前大阶段与审阅目标不一致：当前为「{}」，目标为「{}」。",
            proj.current_milestone_id, milestone_id
        ));
    }

    let expected_mode = if crate::workload_policy::current_profile(proj)?.use_mid_stage_layer {
        project::StageMode::Professional
    } else {
        project::StageMode::Quick
    };
    let milestone = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| format!("大阶段不存在：{}", milestone_id))?;
    if milestone.mode != expected_mode {
        return Err(format!(
            "大阶段拓扑与工作负载画像矛盾：画像要求 {:?}，当前为 {:?}",
            expected_mode, milestone.mode
        ));
    }

    match milestone.mode {
        project::StageMode::Quick => {
            if !proj.current_mid_stage_id.is_empty() {
                return Err("Quick 大阶段审阅必须保持 current_mid_stage_id 为空。".to_string());
            }
            if !milestone.mid_stages.is_empty() {
                return Err("Quick 大阶段不能包含中阶段。".to_string());
            }
            if milestone.subtasks.is_empty() {
                return Err("当前大阶段没有直挂执行任务。".to_string());
            }
            if !milestone
                .subtasks
                .iter()
                .all(|task| is_terminal_subtask(&task.status))
            {
                return Err("大阶段尚有未完成的直挂任务，无法进入审阅。".to_string());
            }
        }
        project::StageMode::Professional => {
            if !milestone.subtasks.is_empty() {
                return Err("Professional 大阶段不能包含直挂执行任务。".to_string());
            }
            if milestone.mid_stages.is_empty() {
                return Err("当前大阶段没有中阶段。".to_string());
            }
            if !proj.current_mid_stage_id.is_empty()
                && !milestone
                    .mid_stages
                    .iter()
                    .any(|stage| stage.id == proj.current_mid_stage_id)
            {
                return Err("当前中阶段不属于审阅目标大阶段。".to_string());
            }
            if milestone
                .mid_stages
                .iter()
                .any(|stage| stage.status != project::MidStageStatus::Completed)
            {
                return Err("大阶段尚有未完成的中阶段，无法进入审阅。".to_string());
            }
            if milestone.mid_stages.iter().any(|stage| {
                stage.subtasks.is_empty()
                    || !stage
                        .subtasks
                        .iter()
                        .all(|task| is_terminal_subtask(&task.status))
            }) {
                return Err("大阶段中仍有缺失或未达到终态的执行任务，无法进入审阅。".to_string());
            }
        }
    }

    let milestone_title = milestone.title.clone();
    let milestone = proj
        .milestones
        .iter_mut()
        .find(|milestone| milestone.id == milestone_id)
        .ok_or_else(|| format!("大阶段不存在：{}", milestone_id))?;
    milestone.status = project::MilestoneStatus::Completed;
    milestone.review_status = Some("pending_review".to_string());
    milestone.review_conclusion = None;
    milestone.human_review_fingerprint = project::milestone_human_review_fingerprint(milestone);
    proj.workflow_state.top_level_phase = project::TopLevelPhase::Console;
    proj.workflow_state.current_step = project::WorkflowStep::MilestoneReview;
    proj.workflow_state.review_node_id = milestone_id.to_string();
    if proj.workflow_state.autopilot_active {
        let autopilot = proj
            .workflow_state
            .autopilot_state
            .get_or_insert_with(project::AutopilotState::default);
        autopilot.active = true;
        autopilot.target_milestone_id = milestone_id.to_string();
        autopilot.run_status = project::AutopilotRunStatus::WaitingMilestoneReview;
        autopilot.last_action = format!("到达大阶段边界：{}，等待人工 A/B/C", milestone_title);
        autopilot.last_action_at = now.to_string();
        autopilot.error_message.clear();
    }
    Ok(())
}

pub(crate) fn has_subtask_execution_facts(subtasks: &[project::Subtask]) -> bool {
    subtasks.iter().any(|subtask| {
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

pub(crate) fn has_plan_execution_facts(mid_stage: &project::MidStage) -> bool {
    !mid_stage.git_tag.is_empty() || has_subtask_execution_facts(&mid_stage.subtasks)
}

pub(crate) fn resolve_direct_milestone_step(
    milestone: &project::Milestone,
) -> Result<project::WorkflowStep, String> {
    if milestone.mode != project::StageMode::Quick {
        return Err("只有 Quick 大阶段可以解析直挂执行计划。".to_string());
    }
    if !milestone.mid_stages.is_empty() {
        return Err("Quick 大阶段不能包含中阶段。".to_string());
    }
    if milestone.status == project::MilestoneStatus::Completed {
        return Ok(project::WorkflowStep::MilestoneReview);
    }
    if milestone.status == project::MilestoneStatus::Paused {
        return Err(format!(
            "大阶段「{}」已暂停，需要人工对账后才能继续。",
            milestone.title
        ));
    }

    let next_step = if milestone.status == project::MilestoneStatus::InProgress
        || has_subtask_execution_facts(&milestone.subtasks)
        || milestone.plan_approved_at.is_some()
        || milestone.plan_revision > 0
    {
        project::WorkflowStep::Execution
    } else if milestone
        .plan_check_result
        .as_ref()
        .is_some_and(|result| result.passed)
    {
        project::WorkflowStep::PlanApproving
    } else if milestone.plan_check_result.is_some()
        || milestone.plan_generated_at.is_some()
        || !milestone.subtasks.is_empty()
    {
        project::WorkflowStep::PlanCheck
    } else {
        project::WorkflowStep::PlanGeneration
    };
    Ok(next_step)
}

pub(crate) fn execution_recovery_selection_step(
    project: &project::Project,
) -> project::WorkflowStep {
    project
        .milestones
        .iter()
        .find(|milestone| milestone.id == project.current_milestone_id)
        .map(|milestone| {
            if milestone.mode == project::StageMode::Quick {
                project::WorkflowStep::MilestoneSelection
            } else {
                project::WorkflowStep::MidStageSelection
            }
        })
        .unwrap_or(project::WorkflowStep::MilestoneSelection)
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
    now: &str,
) -> Result<bool, String> {
    let original_step = proj.workflow_state.current_step.clone();
    let original_mid_stage_id = proj.current_mid_stage_id.clone();
    let mut review_boundary_applied = false;
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
            let milestone_id = proj.current_milestone_id.clone();
            apply_milestone_review_boundary(proj, &milestone_id, now)?;
            review_boundary_applied = true;
        }
        MidStageRoute::WaitHuman { reason } => return Err(reason.clone()),
    }
    Ok(review_boundary_applied
        || original_step != proj.workflow_state.current_step
        || original_mid_stage_id != proj.current_mid_stage_id)
}

pub(crate) fn reconcile_mid_stage_route(
    proj: &mut project::Project,
    now: &str,
) -> Result<bool, String> {
    if proj.workflow_state.current_step != project::WorkflowStep::MilestoneSelection
        || proj.current_milestone_id.is_empty()
    {
        return Ok(false);
    }
    let Some(milestone) = proj
        .milestones
        .iter()
        .find(|milestone| milestone.id == proj.current_milestone_id)
    else {
        return Ok(false);
    };
    let changed = if milestone.mode == project::StageMode::Quick {
        let next_step = resolve_direct_milestone_step(milestone)?;
        if next_step == project::WorkflowStep::MilestoneReview {
            let milestone_id = proj.current_milestone_id.clone();
            apply_milestone_review_boundary(proj, &milestone_id, now)?;
            true
        } else {
            let changed = proj.workflow_state.current_step != next_step
                || !proj.current_mid_stage_id.is_empty();
            proj.current_mid_stage_id.clear();
            proj.workflow_state.current_step = next_step;
            changed
        }
    } else {
        let route = resolve_mid_stage_route(milestone);
        apply_mid_stage_route(proj, &route, now)?
    };
    Ok(changed)
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
            ..Default::default()
        }
    }

    fn terminal_subtasks() -> Vec<project::Subtask> {
        [
            project::SubtaskStatus::Passed,
            project::SubtaskStatus::AcceptedDeviation,
            project::SubtaskStatus::Skipped,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, status)| project::Subtask {
            id: format!("task-{}", index + 1),
            status,
            ..Default::default()
        })
        .collect()
    }

    fn review_boundary_project(mode: project::StageMode) -> project::Project {
        let mut proj = project::Project::new("review-boundary");
        proj.workload_profile = Some(crate::workload_policy::test_profile(match mode {
            project::StageMode::Quick => project::WorkloadScale::Small,
            project::StageMode::Professional => project::WorkloadScale::System,
        }));
        proj.current_milestone_id = "milestone-1".to_string();
        proj.workflow_state.current_step = project::WorkflowStep::Execution;
        proj.workflow_state.data_revision = 7;
        proj.workflow_state.last_transition_at = "previous-transition".to_string();
        proj.workflow_state.autopilot_active = true;
        proj.workflow_state.autopilot_state = Some(project::AutopilotState::default());

        let mut target = milestone(vec![]);
        target.mode = mode.clone();
        target.review_status = Some("approved".to_string());
        target.review_conclusion = Some("A".to_string());
        match mode {
            project::StageMode::Quick => {
                target.subtasks = terminal_subtasks();
                proj.current_mid_stage_id.clear();
            }
            project::StageMode::Professional => {
                let mut completed = mid_stage("mid-1", project::MidStageStatus::Completed, 1);
                completed.subtasks = terminal_subtasks();
                target.mid_stages = vec![completed];
                proj.current_mid_stage_id = "mid-1".to_string();
            }
        }
        proj.milestones = vec![target];
        proj
    }

    fn assert_complete_review_boundary(proj: &project::Project, now: &str) {
        assert_eq!(
            proj.workflow_state.current_step,
            project::WorkflowStep::MilestoneReview
        );
        assert_eq!(proj.workflow_state.review_node_id, "milestone-1");
        assert_eq!(
            proj.milestones[0].status,
            project::MilestoneStatus::Completed
        );
        assert_eq!(
            proj.milestones[0].review_status.as_deref(),
            Some("pending_review")
        );
        assert!(proj.milestones[0].review_conclusion.is_none());
        let autopilot = proj
            .workflow_state
            .autopilot_state
            .as_ref()
            .expect("autopilot boundary");
        assert!(autopilot.active);
        assert_eq!(
            autopilot.run_status,
            project::AutopilotRunStatus::WaitingMilestoneReview
        );
        assert_eq!(autopilot.target_milestone_id, "milestone-1");
        assert_eq!(autopilot.last_action_at, now);
        assert_eq!(proj.workflow_state.data_revision, 7);
        assert_eq!(
            proj.workflow_state.last_transition_at,
            "previous-transition"
        );
    }

    #[test]
    fn adaptive_execution_contract_review_boundary_accepts_all_quick_terminal_outcomes() {
        let mut proj = review_boundary_project(project::StageMode::Quick);
        let now = "2026-08-06T01:02:03Z";
        apply_milestone_review_boundary(&mut proj, "milestone-1", now)
            .expect("Quick review boundary");
        assert_complete_review_boundary(&proj, now);
        assert!(proj.current_mid_stage_id.is_empty());
    }

    #[test]
    fn adaptive_execution_contract_review_boundary_matches_professional_state() {
        let mut proj = review_boundary_project(project::StageMode::Professional);
        let now = "2026-08-06T01:02:03Z";
        apply_milestone_review_boundary(&mut proj, "milestone-1", now)
            .expect("Professional review boundary");
        assert_complete_review_boundary(&proj, now);
        assert_eq!(proj.current_mid_stage_id, "mid-1");
    }

    #[test]
    fn adaptive_execution_contract_review_boundary_rejects_profile_errors_without_mutation() {
        let mut missing = review_boundary_project(project::StageMode::Quick);
        missing.workload_profile = None;
        let error = apply_milestone_review_boundary(&mut missing, "milestone-1", "now")
            .expect_err("missing profile must fail");
        assert!(error.contains("画像缺失"));
        assert_eq!(
            missing.workflow_state.current_step,
            project::WorkflowStep::Execution
        );
        assert_eq!(
            missing.milestones[0].status,
            project::MilestoneStatus::InProgress
        );
        assert_eq!(
            missing.milestones[0].review_status.as_deref(),
            Some("approved")
        );

        let mut stale = review_boundary_project(project::StageMode::Quick);
        stale.discussion_revision = 1;
        let error = apply_milestone_review_boundary(&mut stale, "milestone-1", "now")
            .expect_err("stale profile must fail");
        assert!(error.contains("画像已过期"));
        assert_eq!(stale.workflow_state.data_revision, 7);
    }

    #[test]
    fn adaptive_execution_contract_review_boundary_rejects_topology_and_nonterminal_tasks() {
        let mut topology = review_boundary_project(project::StageMode::Professional);
        topology.workload_profile = Some(crate::workload_policy::test_profile(
            project::WorkloadScale::Small,
        ));
        let error = apply_milestone_review_boundary(&mut topology, "milestone-1", "now")
            .expect_err("profile topology mismatch must fail");
        assert!(error.contains("拓扑"));

        let mut nonterminal = review_boundary_project(project::StageMode::Quick);
        nonterminal.milestones[0].subtasks[0].status = project::SubtaskStatus::Pending;
        let error = apply_milestone_review_boundary(&mut nonterminal, "milestone-1", "now")
            .expect_err("nonterminal task must fail");
        assert!(error.contains("未完成"));
        assert_eq!(
            nonterminal.workflow_state.current_step,
            project::WorkflowStep::Execution
        );
        assert_eq!(nonterminal.workflow_state.data_revision, 7);
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

    #[test]
    fn quick_milestone_routes_directly_to_plan_generation() {
        let mut direct = milestone(vec![]);
        direct.mode = project::StageMode::Quick;
        direct.status = project::MilestoneStatus::Pending;
        assert_eq!(
            resolve_direct_milestone_step(&direct).unwrap(),
            project::WorkflowStep::PlanGeneration
        );

        let mut project = project::Project::new("quick-route");
        project.current_milestone_id = direct.id.clone();
        project.workflow_state.current_step = project::WorkflowStep::MilestoneSelection;
        project.milestones.push(direct);
        assert!(reconcile_mid_stage_route(&mut project, "2026-08-06T00:00:00Z").unwrap());
        assert_eq!(
            project.workflow_state.current_step,
            project::WorkflowStep::PlanGeneration
        );
        assert!(project.current_mid_stage_id.is_empty());
    }

    #[test]
    fn quick_approved_plan_routes_to_execution_without_mid_stage() {
        let mut direct = milestone(vec![]);
        direct.mode = project::StageMode::Quick;
        direct.plan_approved_at = Some("2026-08-06T00:00:00Z".to_string());
        direct.plan_revision = 3;
        assert_eq!(
            resolve_direct_milestone_step(&direct).unwrap(),
            project::WorkflowStep::Execution
        );
    }

    #[test]
    fn execution_recovery_returns_to_the_topology_selection_boundary() {
        let mut quick = project::Project::new("quick-recovery");
        quick.current_milestone_id = "milestone-1".to_string();
        let mut quick_milestone = milestone(vec![]);
        quick_milestone.mode = project::StageMode::Quick;
        quick.milestones.push(quick_milestone);
        assert_eq!(
            execution_recovery_selection_step(&quick),
            project::WorkflowStep::MilestoneSelection
        );

        let mut professional = project::Project::new("professional-recovery");
        professional.current_milestone_id = "milestone-1".to_string();
        professional.milestones.push(milestone(vec![mid_stage(
            "mid",
            project::MidStageStatus::InProgress,
            1,
        )]));
        assert_eq!(
            execution_recovery_selection_step(&professional),
            project::WorkflowStep::MidStageSelection
        );
    }
}
