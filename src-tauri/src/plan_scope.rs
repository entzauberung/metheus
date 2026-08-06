use crate::project;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanTargetKind {
    Milestone,
    MidStage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlanScope {
    milestone_index: usize,
    mid_stage_index: Option<usize>,
}

impl PlanScope {
    pub(crate) fn resolve(project: &project::Project) -> Result<Self, String> {
        let profile = crate::workload_policy::current_profile(project)?;
        if project.current_milestone_id.is_empty() {
            return Err("当前计划缺少已选择的大阶段。".to_string());
        }
        let milestone_index = project
            .milestones
            .iter()
            .position(|milestone| milestone.id == project.current_milestone_id)
            .ok_or_else(|| "当前计划选择的大阶段不存在。".to_string())?;
        let milestone = &project.milestones[milestone_index];
        let expected_mode = if profile.use_mid_stage_layer {
            project::StageMode::Professional
        } else {
            project::StageMode::Quick
        };
        if milestone.mode != expected_mode {
            return Err(format!(
                "大阶段拓扑与工作负载画像矛盾：画像要求 {:?}，当前为 {:?}",
                expected_mode, milestone.mode
            ));
        }

        match milestone.mode {
            project::StageMode::Quick => {
                if !project.current_mid_stage_id.is_empty() {
                    return Err("Quick 计划必须保持 current_mid_stage_id 为空。".to_string());
                }
                if !milestone.mid_stages.is_empty() {
                    return Err("Quick 大阶段不能同时包含中阶段与直挂任务容器。".to_string());
                }
                Ok(Self {
                    milestone_index,
                    mid_stage_index: None,
                })
            }
            project::StageMode::Professional => {
                if project.current_mid_stage_id.is_empty() {
                    return Err("Professional 计划必须选择有效的中阶段。".to_string());
                }
                if !milestone.subtasks.is_empty() {
                    return Err(
                        "Professional 大阶段不能同时包含直挂任务与中阶段任务容器。".to_string()
                    );
                }
                let mid_stage_index = milestone
                    .mid_stages
                    .iter()
                    .position(|mid_stage| mid_stage.id == project.current_mid_stage_id)
                    .ok_or_else(|| "当前计划选择的中阶段不存在。".to_string())?;
                Ok(Self {
                    milestone_index,
                    mid_stage_index: Some(mid_stage_index),
                })
            }
        }
    }

    pub(crate) fn kind(self) -> PlanTargetKind {
        if self.mid_stage_index.is_some() {
            PlanTargetKind::MidStage
        } else {
            PlanTargetKind::Milestone
        }
    }

    pub(crate) fn milestone<'a>(self, project: &'a project::Project) -> &'a project::Milestone {
        &project.milestones[self.milestone_index]
    }

    pub(crate) fn milestone_mut<'a>(
        self,
        project: &'a mut project::Project,
    ) -> &'a mut project::Milestone {
        &mut project.milestones[self.milestone_index]
    }

    pub(crate) fn mid_stage<'a>(
        self,
        project: &'a project::Project,
    ) -> Option<&'a project::MidStage> {
        self.mid_stage_index
            .map(|index| &project.milestones[self.milestone_index].mid_stages[index])
    }

    pub(crate) fn mid_stage_id(self, project: &project::Project) -> String {
        self.mid_stage(project)
            .map(|stage| stage.id.clone())
            .unwrap_or_default()
    }

    pub(crate) fn target_id<'a>(self, project: &'a project::Project) -> &'a str {
        self.mid_stage(project)
            .map(|stage| stage.id.as_str())
            .unwrap_or_else(|| self.milestone(project).id.as_str())
    }

    pub(crate) fn subtasks<'a>(self, project: &'a project::Project) -> &'a [project::Subtask] {
        match self.mid_stage_index {
            Some(index) => &project.milestones[self.milestone_index].mid_stages[index].subtasks,
            None => &project.milestones[self.milestone_index].subtasks,
        }
    }

    pub(crate) fn subtasks_mut<'a>(
        self,
        project: &'a mut project::Project,
    ) -> &'a mut Vec<project::Subtask> {
        match self.mid_stage_index {
            Some(index) => &mut project.milestones[self.milestone_index].mid_stages[index].subtasks,
            None => &mut project.milestones[self.milestone_index].subtasks,
        }
    }

    pub(crate) fn has_execution_facts(self, project: &project::Project) -> bool {
        match self.mid_stage(project) {
            Some(mid_stage) => crate::workflow_resolution::has_plan_execution_facts(mid_stage),
            None => crate::workflow_resolution::has_subtask_execution_facts(self.subtasks(project)),
        }
    }

    pub(crate) fn plan_check_result<'a>(
        self,
        project: &'a project::Project,
    ) -> Option<&'a project::StagePlanCheckResult> {
        match self.mid_stage_index {
            Some(index) => project.milestones[self.milestone_index].mid_stages[index]
                .plan_check_result
                .as_ref(),
            None => project.milestones[self.milestone_index]
                .plan_check_result
                .as_ref(),
        }
    }

    pub(crate) fn plan_check_result_mut<'a>(
        self,
        project: &'a mut project::Project,
    ) -> Option<&'a mut project::StagePlanCheckResult> {
        match self.mid_stage_index {
            Some(index) => project.milestones[self.milestone_index].mid_stages[index]
                .plan_check_result
                .as_mut(),
            None => project.milestones[self.milestone_index]
                .plan_check_result
                .as_mut(),
        }
    }

    pub(crate) fn plan_approved_at<'a>(self, project: &'a project::Project) -> Option<&'a str> {
        match self.mid_stage_index {
            Some(index) => project.milestones[self.milestone_index].mid_stages[index]
                .plan_approved_at
                .as_deref(),
            None => project.milestones[self.milestone_index]
                .plan_approved_at
                .as_deref(),
        }
    }

    pub(crate) fn plan_revision(self, project: &project::Project) -> u64 {
        match self.mid_stage_index {
            Some(index) => project.milestones[self.milestone_index].mid_stages[index].plan_revision,
            None => project.milestones[self.milestone_index].plan_revision,
        }
    }

    pub(crate) fn plan_draft_revision(self, project: &project::Project) -> u64 {
        match self.mid_stage_index {
            Some(index) => {
                project.milestones[self.milestone_index].mid_stages[index].plan_draft_revision
            }
            None => project.milestones[self.milestone_index].plan_draft_revision,
        }
    }

    pub(crate) fn plan_generated_at<'a>(self, project: &'a project::Project) -> Option<&'a str> {
        match self.mid_stage_index {
            Some(index) => project.milestones[self.milestone_index].mid_stages[index]
                .plan_generated_at
                .as_deref(),
            None => project.milestones[self.milestone_index]
                .plan_generated_at
                .as_deref(),
        }
    }

    pub(crate) fn plan_regeneration_count(self, project: &project::Project) -> u32 {
        match self.mid_stage_index {
            Some(index) => {
                project.milestones[self.milestone_index].mid_stages[index].plan_regeneration_count
            }
            None => project.milestones[self.milestone_index].plan_regeneration_count,
        }
    }

    pub(crate) fn last_plan_failure_fingerprint<'a>(
        self,
        project: &'a project::Project,
    ) -> &'a str {
        match self.mid_stage_index {
            Some(index) => {
                &project.milestones[self.milestone_index].mid_stages[index]
                    .last_plan_failure_fingerprint
            }
            None => &project.milestones[self.milestone_index].last_plan_failure_fingerprint,
        }
    }

    pub(crate) fn last_plan_issue_count(self, project: &project::Project) -> u32 {
        match self.mid_stage_index {
            Some(index) => {
                project.milestones[self.milestone_index].mid_stages[index].last_plan_issue_count
            }
            None => project.milestones[self.milestone_index].last_plan_issue_count,
        }
    }

    pub(crate) fn plan_no_progress_count(self, project: &project::Project) -> u32 {
        match self.mid_stage_index {
            Some(index) => {
                project.milestones[self.milestone_index].mid_stages[index].plan_no_progress_count
            }
            None => project.milestones[self.milestone_index].plan_no_progress_count,
        }
    }

    pub(crate) fn set_generated_plan(
        self,
        project: &mut project::Project,
        subtasks: Vec<project::Subtask>,
        generated_at: String,
        regeneration_count: u32,
        no_progress_count: u32,
    ) {
        match self.mid_stage_index {
            Some(index) => {
                let target = &mut project.milestones[self.milestone_index].mid_stages[index];
                target.subtasks = subtasks;
                target.plan_check_result = None;
                target.plan_approved_at = None;
                target.plan_revision = 0;
                target.plan_draft_revision = target.plan_draft_revision.saturating_add(1);
                target.plan_generated_at = Some(generated_at);
                target.plan_regeneration_count = regeneration_count;
                target.last_plan_failure_fingerprint.clear();
                target.last_plan_issue_count = 0;
                target.plan_no_progress_count = no_progress_count;
            }
            None => {
                let target = &mut project.milestones[self.milestone_index];
                target.subtasks = subtasks;
                target.plan_check_result = None;
                target.plan_approved_at = None;
                target.plan_revision = 0;
                target.plan_draft_revision = target.plan_draft_revision.saturating_add(1);
                target.plan_generated_at = Some(generated_at);
                target.plan_regeneration_count = regeneration_count;
                target.last_plan_failure_fingerprint.clear();
                target.last_plan_issue_count = 0;
                target.plan_no_progress_count = no_progress_count;
            }
        }
    }

    pub(crate) fn set_plan_check_result(
        self,
        project: &mut project::Project,
        result: project::StagePlanCheckResult,
        failure_fingerprint: String,
        issue_count: u32,
        no_progress_count: u32,
    ) {
        match self.mid_stage_index {
            Some(index) => {
                let target = &mut project.milestones[self.milestone_index].mid_stages[index];
                target.plan_check_result = Some(result);
                target.last_plan_failure_fingerprint = failure_fingerprint;
                target.last_plan_issue_count = issue_count;
                target.plan_no_progress_count = no_progress_count;
            }
            None => {
                let target = &mut project.milestones[self.milestone_index];
                target.plan_check_result = Some(result);
                target.last_plan_failure_fingerprint = failure_fingerprint;
                target.last_plan_issue_count = issue_count;
                target.plan_no_progress_count = no_progress_count;
            }
        }
    }

    pub(crate) fn approve_plan(
        self,
        project: &mut project::Project,
        approved_at: String,
        plan_revision: u64,
    ) {
        match self.mid_stage_index {
            Some(index) => {
                let target = &mut project.milestones[self.milestone_index].mid_stages[index];
                target.plan_approved_at = Some(approved_at);
                target.plan_revision = plan_revision;
                target.status = project::MidStageStatus::InProgress;
            }
            None => {
                let target = &mut project.milestones[self.milestone_index];
                target.plan_approved_at = Some(approved_at);
                target.plan_revision = plan_revision;
                target.status = project::MilestoneStatus::InProgress;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(use_mid_stage_layer: bool) -> project::WorkloadProfile {
        let mut signals = project::WorkloadSignals {
            has_frontend: true,
            has_backend: use_mid_stage_layer,
            has_persistence: false,
            has_auth_or_roles: false,
            external_integration_count: 0,
            independent_domain_count: if use_mid_stage_layer { 3 } else { 1 },
            deliverable_count: if use_mid_stage_layer { 3 } else { 2 },
            high_risk: false,
        };
        if use_mid_stage_layer {
            signals.has_frontend = true;
        }
        crate::workload_policy::classify(signals, None, 0).unwrap()
    }

    fn milestone(mode: project::StageMode) -> project::Milestone {
        project::Milestone {
            id: "milestone-1".to_string(),
            mode,
            ..test_milestone_defaults()
        }
    }

    fn test_milestone_defaults() -> project::Milestone {
        project::Milestone {
            id: String::new(),
            version: "v0.1".to_string(),
            title: "test".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: project::MilestoneStatus::Pending,
            mode: project::StageMode::Quick,
            mid_stages: vec![],
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

    #[test]
    fn adaptive_execution_contract_quick_resolves_direct_tasks() {
        let mut project = project::Project::new("quick");
        project.workload_profile = Some(profile(false));
        project.current_milestone_id = "milestone-1".to_string();
        project
            .milestones
            .push(milestone(project::StageMode::Quick));
        let scope = PlanScope::resolve(&project).unwrap();
        assert_eq!(scope.kind(), PlanTargetKind::Milestone);
        assert!(scope.mid_stage_id(&project).is_empty());
    }

    #[test]
    fn adaptive_execution_contract_professional_requires_mid_stage() {
        let mut project = project::Project::new("professional");
        project.workload_profile = Some(profile(true));
        project.current_milestone_id = "milestone-1".to_string();
        project
            .milestones
            .push(milestone(project::StageMode::Professional));
        assert!(PlanScope::resolve(&project)
            .unwrap_err()
            .contains("必须选择有效的中阶段"));
    }

    #[test]
    fn mixed_containers_are_rejected() {
        let mut project = project::Project::new("mixed");
        project.workload_profile = Some(profile(false));
        project.current_milestone_id = "milestone-1".to_string();
        let mut milestone = milestone(project::StageMode::Quick);
        milestone.mid_stages.push(project::MidStage {
            id: "mid-1".to_string(),
            title: String::new(),
            version: String::new(),
            order: None,
            status: project::MidStageStatus::Pending,
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
        });
        project.milestones.push(milestone);
        assert!(PlanScope::resolve(&project)
            .unwrap_err()
            .contains("不能同时包含中阶段"));
    }
}
