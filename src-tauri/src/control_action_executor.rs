use crate::control_action::{ControlActionKind, ControlActionLifecycle};
use crate::pipeline::PipelineState;
use crate::project;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::Mutex;

pub fn ensure_serial_takeover_actions_available() -> Result<(), String> {
    use ControlActionKind::*;
    let required = [
        Split,
        Execute,
        LocalValidate,
        AutomatedValidate,
        TargetedValidate,
        Repair,
        Recompile,
        AcceptDeviation,
        GitConfirm,
        Wait,
        Human,
    ];
    if required.iter().all(|action| {
        matches!(
            action,
            Split
                | Execute
                | LocalValidate
                | AutomatedValidate
                | TargetedValidate
                | Repair
                | Recompile
                | AcceptDeviation
                | GitConfirm
                | Wait
                | Human
        )
    }) {
        Ok(())
    } else {
        Err("串行接管动作执行器覆盖不完整".to_string())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ControlActionRequest {
    #[serde(default)]
    pub action_id: String,
    pub action: ControlActionKind,
    #[serde(default)]
    pub task_id: String,
    #[serde(default)]
    pub decision_id: String,
    #[serde(default)]
    pub expected_project_revision: Option<u64>,
    #[serde(default)]
    pub expected_tree_revision: Option<u64>,
    #[serde(default)]
    pub contract_fingerprint: String,
    #[serde(default)]
    pub criterion_indexes: Vec<u32>,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub source: project::OperationSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlActionExecutionResult {
    pub action_id: String,
    pub action: ControlActionKind,
    pub task_id: String,
    pub lifecycle: ControlActionLifecycle,
    pub idempotent: bool,
    pub queued: bool,
    pub made_progress: bool,
    pub before_fingerprint: String,
    pub after_fingerprint: String,
    pub project_revision: u64,
    pub tree_revision: u64,
    pub message: String,
}

pub async fn execute(
    pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    project_name: String,
    mut request: ControlActionRequest,
) -> Result<ControlActionExecutionResult, String> {
    if request.action_id.trim().is_empty() {
        request.action_id = uuid::Uuid::new_v4().to_string();
    }
    let mut project = crate::load_project(&project_name)?;
    validate_request(&project, &request)?;
    if project.task_control.last_completed_action_id == request.action_id {
        return Ok(previous_result(&project, &request));
    }
    if !project.task_control.active_action_id.is_empty() {
        if project.task_control.active_action_id == request.action_id {
            return Ok(ControlActionExecutionResult {
                action_id: request.action_id,
                action: request.action,
                task_id: request.task_id,
                lifecycle: ControlActionLifecycle::Running,
                idempotent: true,
                queued: true,
                made_progress: false,
                before_fingerprint: project.task_control.last_action_before_fingerprint.clone(),
                after_fingerprint: String::new(),
                project_revision: project.workflow_state.data_revision,
                tree_revision: project.task_control.tree_revision,
                message: "同一控制动作正在执行".to_string(),
            });
        }
        return Err(format!(
            "已有控制动作正在执行：{}",
            project.task_control.active_action_id
        ));
    }

    let before_fingerprint = control_fingerprint(&project, &request.task_id)?;
    project.task_control.active_action_id = request.action_id.clone();
    project.task_control.active_action_kind = request.action.as_str().to_string();
    project.task_control.active_action_task_id = request.task_id.clone();
    project.task_control.last_action_before_fingerprint = before_fingerprint.clone();
    project.task_control.last_action_at = Some(chrono::Utc::now().to_rfc3339());
    project.workflow_state.data_revision = project.workflow_state.data_revision.saturating_add(1);
    crate::save_project(&project)?;

    let claimed_project_revision = project.workflow_state.data_revision;
    let claimed_tree_revision = project.task_control.tree_revision;
    let dispatched = dispatch(
        pipeline_state,
        &project_name,
        &request,
        claimed_project_revision,
        claimed_tree_revision,
    )
    .await;
    match dispatched {
        Ok(message) => finish_action(&project_name, &request, before_fingerprint, message, true),
        Err(error) => {
            let _ = finish_action(
                &project_name,
                &request,
                before_fingerprint,
                error.clone(),
                false,
            );
            Err(error)
        }
    }
}

fn validate_request(
    project: &project::Project,
    request: &ControlActionRequest,
) -> Result<(), String> {
    if let Some(expected) = request.expected_project_revision {
        if project.workflow_state.data_revision != expected {
            return Err(format!(
                "项目修订冲突：请求={}，磁盘={}",
                expected, project.workflow_state.data_revision
            ));
        }
    }
    if let Some(expected) = request.expected_tree_revision {
        if project.task_control.tree_revision != expected {
            return Err(format!(
                "任务树修订冲突：请求={}，磁盘={}",
                expected, project.task_control.tree_revision
            ));
        }
    }
    if request.action == ControlActionKind::Wait {
        return Ok(());
    }
    if request.action == ControlActionKind::Human
        && request.task_id.is_empty()
        && request.criterion_indexes.is_empty()
    {
        return Ok(());
    }
    if request.task_id.is_empty() {
        return Err("控制动作必须指定任务节点".to_string());
    }
    let address = crate::task_tree::locate_task(project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    let task = crate::task_tree::find_task(project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    if request.source == project::OperationSource::User {
        let current = crate::task_tree::select_current_leaf(project)?
            .is_some_and(|address| address.task_id == request.task_id);
        let recovery_bound = project
            .workflow_state
            .recovery_state
            .as_ref()
            .zip(project.execution_session.as_ref())
            .is_some_and(|(recovery, session)| {
                recovery.phase == project::RecoveryPhase::WaitingHuman
                    && recovery.subtask_id == request.task_id
                    && session.subtask_id == request.task_id
            });
        if !current && !recovery_bound {
            return Err("只能操作当前叶子或当前人工恢复会话绑定的任务".to_string());
        }
    }
    if !request.contract_fingerprint.is_empty() {
        let contract = crate::task_compiler::compile(
            task,
            address.ancestor_task_ids.last().map(String::as_str),
            address.depth,
        )
        .contract;
        if contract.fingerprint != request.contract_fingerprint {
            return Err("任务合同指纹已变化，拒绝旧控制动作".to_string());
        }
    }
    match request.action {
        ControlActionKind::Split | ControlActionKind::Recompile => {
            if project.execution_session.as_ref().is_some_and(|session| {
                session.active
                    && (session.subtask_id == task.id
                        || session.task_path.iter().any(|id| id == &task.id))
            }) {
                return Err("执行中的任务及其祖先不能拆分或重编译".to_string());
            }
            if crate::task_tree::is_terminal(&task.status) {
                return Err("已完成任务不能拆分或重编译".to_string());
            }
        }
        ControlActionKind::Execute => {
            if !task.child_tasks.is_empty() || task.status != project::SubtaskStatus::Pending {
                return Err("执行动作只能作用于 Pending 叶子任务".to_string());
            }
            if !address.dependencies_satisfied {
                return Err("叶子任务依赖尚未满足".to_string());
            }
        }
        ControlActionKind::LocalValidate
        | ControlActionKind::AutomatedValidate
        | ControlActionKind::TargetedValidate => {
            if !task.child_tasks.is_empty() || crate::task_tree::is_terminal(&task.status) {
                return Err("验证动作只能作用于未完成叶子任务".to_string());
            }
        }
        ControlActionKind::Human if !request.criterion_indexes.is_empty() => {
            if !task.child_tasks.is_empty() || crate::task_tree::is_terminal(&task.status) {
                return Err("人工审查只能作用于未完成叶子任务".to_string());
            }
            validation_targets_for_mode(
                task,
                &request.criterion_indexes,
                crate::validator_contract::VerificationMode::HumanReview,
            )?;
        }
        ControlActionKind::Repair => {
            if !task
                .acceptance_ledger
                .iter()
                .any(|item| item.status == project::AcceptanceStatus::Unsatisfied)
            {
                return Err("修复动作需要明确未满足的验收证据".to_string());
            }
        }
        ControlActionKind::AcceptDeviation => {
            crate::human_action_policy::authorize(
                project,
                &request.task_id,
                crate::human_action_policy::HumanTerminalAction::AcceptDeviation,
                &request.criterion_indexes,
                &request.reason,
            )?;
        }
        ControlActionKind::GitConfirm => {
            if task.status != project::SubtaskStatus::AwaitingConfirmation {
                return Err("Git 确认只能作用于待确认叶子任务".to_string());
            }
        }
        ControlActionKind::Wait | ControlActionKind::Human => {}
    }
    Ok(())
}

async fn dispatch(
    pipeline_state: Arc<Mutex<Option<PipelineState>>>,
    project_name: &str,
    request: &ControlActionRequest,
    claimed_project_revision: u64,
    claimed_tree_revision: u64,
) -> Result<String, String> {
    let claimed_project = crate::load_project(project_name)?;
    validate_claimed_dispatch(
        &claimed_project,
        request,
        claimed_project_revision,
        claimed_tree_revision,
    )?;
    match request.action {
        ControlActionKind::Split => {
            let mut project = claimed_project;
            crate::commands::task_control::split_task(&mut project, &request.task_id)?;
            project.workflow_state.data_revision =
                project.workflow_state.data_revision.saturating_add(1);
            crate::save_project(&project)?;
            Ok("任务已按独立产物拆分".to_string())
        }
        ControlActionKind::Execute => {
            crate::pipeline::execute_task_with_source(
                pipeline_state,
                project_name.to_string(),
                Some(request.task_id.clone()),
                request.source,
            )
            .await?;
            Ok("叶子任务执行已派发".to_string())
        }
        ControlActionKind::LocalValidate => {
            run_local_validation(project_name, request)?;
            Ok("本地确定性验证已完成".to_string())
        }
        ControlActionKind::AutomatedValidate => {
            let status = run_automated_validation(project_name, request)?;
            Ok(match status {
                project::AutomatedTestStatus::Passed => "自动化测试验证已通过".to_string(),
                project::AutomatedTestStatus::Failed => "自动化测试验证发现失败".to_string(),
                project::AutomatedTestStatus::NotConfigured => {
                    "项目未配置自动化测试，验收项保持未证明".to_string()
                }
                project::AutomatedTestStatus::Unavailable => {
                    "自动化测试环境不可用，已进入人工边界".to_string()
                }
                project::AutomatedTestStatus::Unknown => {
                    "自动化测试状态未知，验收项保持未证明".to_string()
                }
            })
        }
        ControlActionKind::TargetedValidate => {
            run_targeted_validation(project_name, request).await?;
            Ok("定向验证已完成".to_string())
        }
        ControlActionKind::Repair => {
            let mut project = claimed_project;
            let automatic = crate::recovery::ensure_quality_recovery(
                &mut project,
                if request.reason.is_empty() {
                    "控制器发现明确未满足验收项"
                } else {
                    &request.reason
                },
            )?;
            crate::save_project(&project)?;
            if !automatic {
                return Err("当前修复需要人工处理".to_string());
            }
            crate::recovery::run_error_recovery_with_pipeline(
                pipeline_state,
                project_name.to_string(),
            )
            .await?;
            Ok("受限修复已执行".to_string())
        }
        ControlActionKind::Recompile => {
            let mut project = claimed_project;
            crate::commands::task_control::recompile_task(&mut project, &request.task_id)?;
            project.workflow_state.data_revision =
                project.workflow_state.data_revision.saturating_add(1);
            crate::save_project(&project)?;
            Ok("当前任务合同已重编译".to_string())
        }
        ControlActionKind::AcceptDeviation => {
            let mut project = claimed_project;
            crate::commands::task_control::accept_deviation(
                &mut project,
                &request.task_id,
                &request.criterion_indexes,
                &request.reason,
            )?;
            if crate::task_tree::find_task(&project, &request.task_id)?
                .is_some_and(|task| crate::task_tree::is_terminal(&task.status))
            {
                crate::task_aggregation::aggregate_ancestors(&mut project, &request.task_id)?;
            }
            project.workflow_state.data_revision =
                project.workflow_state.data_revision.saturating_add(1);
            crate::save_project_if_revision(
                &project,
                claimed_project_revision,
                claimed_tree_revision,
            )?;
            Ok("验收偏差已按任务和验收项记录".to_string())
        }
        ControlActionKind::GitConfirm => {
            crate::pipeline::confirm_subtask_result_with_source(
                &pipeline_state,
                project_name.to_string(),
                request.source,
            )
            .await?;
            Ok("当前叶子 Git 确认已完成".to_string())
        }
        ControlActionKind::Wait => Ok("控制器等待新的项目事实".to_string()),
        ControlActionKind::Human => {
            let mut project = claimed_project;
            let message = enter_human_boundary(&mut project, request);
            crate::save_project(&project)?;
            Ok(message)
        }
    }
}

fn enter_human_boundary(project: &mut project::Project, request: &ControlActionRequest) -> String {
    let message = if request.criterion_indexes.is_empty() {
        "控制器已进入人工边界".to_string()
    } else {
        format!(
            "验收项 {} 已进入人工审查边界，等待显式人工结论",
            request
                .criterion_indexes
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
        state.run_status = project::AutopilotRunStatus::ErrorStopped;
        state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        state.error_message = if request.reason.trim().is_empty() {
            message.clone()
        } else {
            request.reason.clone()
        };
    }
    message
}

fn validate_claimed_dispatch(
    project: &project::Project,
    request: &ControlActionRequest,
    claimed_project_revision: u64,
    claimed_tree_revision: u64,
) -> Result<(), String> {
    if project.workflow_state.data_revision != claimed_project_revision
        || project.task_control.tree_revision != claimed_tree_revision
    {
        return Err("控制动作认领后项目状态已变化，拒绝旧动作".to_string());
    }
    if project.task_control.active_action_id != request.action_id
        || project.task_control.active_action_kind != request.action.as_str()
        || project.task_control.active_action_task_id != request.task_id
    {
        return Err("控制动作认领已被新的项目状态取代".to_string());
    }
    let mut revalidated = request.clone();
    revalidated.expected_project_revision = Some(claimed_project_revision);
    revalidated.expected_tree_revision = Some(claimed_tree_revision);
    validate_request(project, &revalidated)
}

fn run_local_validation(project_name: &str, request: &ControlActionRequest) -> Result<(), String> {
    let mut project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let authorized = crate::plan_contract::validate_subtask(&task, "本地验证任务")?;
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::Deterministic,
    )?;
    let mut updates = Vec::new();
    for index in targets {
        let criterion = task
            .acceptance_criteria
            .get(index.saturating_sub(1) as usize)
            .ok_or_else(|| format!("验收项不存在：{}", index))?;
        let Some(batch) = crate::validator_registry::try_validate_locally(
            &project.project_path,
            std::slice::from_ref(criterion),
            &authorized,
        ) else {
            updates.push(project::AcceptanceLedgerItem {
                criterion_index: index,
                criterion: criterion.clone(),
                status: project::AcceptanceStatus::Unknown,
                evidence: "local_unprovable:本地验证器无法保守证明，转入定向语义审查".to_string(),
                confidence: 0.0,
                updated_at: chrono::Utc::now().to_rfc3339(),
                ..Default::default()
            });
            continue;
        };
        let Some(review) = batch.criterion_reviews.first() else {
            continue;
        };
        let evidence = batch
            .validator_runs
            .first()
            .map(|run| {
                format!(
                    "{}@{}:{}",
                    run.validator, run.version, run.evidence_fingerprint
                )
            })
            .unwrap_or_else(|| "本地确定性验证".to_string());
        updates.push(project::AcceptanceLedgerItem {
            criterion_index: index,
            criterion: criterion.clone(),
            status: match review.conclusion {
                project::CriterionReviewConclusion::Satisfied => {
                    project::AcceptanceStatus::Satisfied
                }
                project::CriterionReviewConclusion::Unsatisfied => {
                    project::AcceptanceStatus::Unsatisfied
                }
                project::CriterionReviewConclusion::EvidenceInsufficient => {
                    project::AcceptanceStatus::Unknown
                }
            },
            evidence,
            evidence_references: review.evidence_references.clone(),
            confidence: review.confidence,
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    merge_ledger_updates(&mut project, &request.task_id, updates)?;
    crate::save_project(&project)
}

fn validation_targets_for_mode(
    task: &project::Subtask,
    requested: &[u32],
    mode: crate::validator_contract::VerificationMode,
) -> Result<Vec<u32>, String> {
    let candidates = crate::acceptance::revalidation_target_indexes(task, requested)?;
    let targets = candidates
        .iter()
        .copied()
        .filter(|index| {
            let configured = crate::validator_registry::verification_mode_for(task, *index);
            configured == mode
                || (mode == crate::validator_contract::VerificationMode::SemanticReview
                    && configured == crate::validator_contract::VerificationMode::Deterministic
                    && task.acceptance_ledger.iter().any(|item| {
                        item.criterion_index == *index
                            && item.status == project::AcceptanceStatus::Unknown
                            && item.evidence.starts_with("local_unprovable:")
                    }))
        })
        .collect::<Vec<_>>();
    if !requested.is_empty() && targets.len() != candidates.len() {
        return Err(format!("请求包含不属于 {:?} 通道的验收项", mode));
    }
    if targets.is_empty() {
        return Err(format!("当前任务没有需要 {:?} 验证的验收项", mode));
    }
    Ok(targets)
}

fn automated_ledger_updates(
    task: &project::Subtask,
    targets: &[u32],
    evidence: &crate::automated_validation::AutomatedTestEvidence,
) -> Result<Vec<project::AcceptanceLedgerItem>, String> {
    let mut updates = Vec::new();
    for index in targets {
        if crate::validator_registry::verification_mode_for(task, *index)
            != crate::validator_contract::VerificationMode::AutomatedTest
        {
            return Err(format!("验收项 {} 不是自动化测试验证模式", index));
        }
        let criterion = task
            .acceptance_criteria
            .get(index.saturating_sub(1) as usize)
            .ok_or_else(|| format!("验收项不存在：{}", index))?;
        let status = match evidence.status {
            project::AutomatedTestStatus::Passed => project::AcceptanceStatus::Satisfied,
            project::AutomatedTestStatus::Failed => project::AcceptanceStatus::Unsatisfied,
            project::AutomatedTestStatus::NotConfigured
            | project::AutomatedTestStatus::Unavailable
            | project::AutomatedTestStatus::Unknown => project::AcceptanceStatus::Unknown,
        };
        updates.push(project::AcceptanceLedgerItem {
            criterion_index: *index,
            criterion: criterion.clone(),
            status,
            evidence: format!(
                "automated_test_runner: command={} status={:?} exit_code={:?}; {}",
                evidence.command, evidence.status, evidence.exit_code, evidence.output_summary
            ),
            evidence_references: vec![],
            confidence: if matches!(
                evidence.status,
                project::AutomatedTestStatus::Passed | project::AutomatedTestStatus::Failed
            ) {
                1.0
            } else {
                0.0
            },
            updated_at: chrono::Utc::now().to_rfc3339(),
        });
    }
    Ok(updates)
}

fn run_automated_validation(
    project_name: &str,
    request: &ControlActionRequest,
) -> Result<project::AutomatedTestStatus, String> {
    let project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::AutomatedTest,
    )?;
    let evidence = crate::automated_validation::run_project_tests(&project.project_path);
    let updates = automated_ledger_updates(&task, &targets, &evidence)?;
    let mut project = crate::load_project(project_name)?;
    merge_ledger_updates(&mut project, &request.task_id, updates)?;
    let task = crate::task_tree::find_task_mut(&mut project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?;
    let mut result = task.test_result.clone().unwrap_or_default();
    result.test_command = evidence.command.clone();
    result.test_exit_code = evidence.exit_code;
    result.test_output_summary = evidence.output_summary.clone();
    result.automated_test_status = evidence.status.clone();
    result.verification_kind = project::VerificationKind::AutomatedTestOnly;
    result.acceptance_results = task.acceptance_ledger.clone();
    result.passed = evidence.status == project::AutomatedTestStatus::Passed;
    task.test_result = Some(result);
    if evidence.status == project::AutomatedTestStatus::Unavailable {
        let state = project
            .workflow_state
            .autopilot_state
            .get_or_insert_with(project::AutopilotState::default);
        state.run_status = project::AutopilotRunStatus::ErrorStopped;
        state.recovery_action = project::AutopilotRecoveryAction::WaitHumanDecision;
        state.error_message = "自动化测试环境不可用，不代表代码失败".to_string();
    }
    crate::save_project(&project)?;
    Ok(evidence.status)
}

async fn run_targeted_validation(
    project_name: &str,
    request: &ControlActionRequest,
) -> Result<(), String> {
    let project = crate::load_project(project_name)?;
    let task = crate::task_tree::find_task(&project, &request.task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", request.task_id))?
        .clone();
    let authorized = crate::plan_contract::validate_subtask(&task, "定向验证任务")?;
    let targets = validation_targets_for_mode(
        &task,
        &request.criterion_indexes,
        crate::validator_contract::VerificationMode::SemanticReview,
    )?;
    let previous_test = task.test_result.clone().unwrap_or_default();
    let result = crate::test_runner::review_subtask_with_context_and_model(
        &project.project_path,
        if task.goal.is_empty() {
            &task.title
        } else {
            &task.goal
        },
        &task.id,
        &project.current_milestone_id,
        &project.current_mid_stage_id,
        Some(task.acceptance_criteria.clone()),
        Some(authorized.clone()),
        Some(crate::plan_compiler::compile_execution_prompt(&task)),
        Some(crate::review_evidence::ReviewEvidenceRequest::for_task(
            &task,
            project::ReviewEvidenceStrategy::Targeted,
            targets.clone(),
        )),
        &previous_test,
        Some(crate::cost_ledger::ModelCallContext {
            project_name: project.name.clone(),
            milestone_id: project.current_milestone_id.clone(),
            stage_id: project.current_mid_stage_id.clone(),
            task_id: task.id.clone(),
            purpose: Some(crate::cost_ledger::ModelCallPurpose::EvidenceSupplement),
            decision_id: request.decision_id.clone(),
            action_id: request.action_id.clone(),
        }),
    )
    .await?;
    let ledger = crate::acceptance::build_ledger(&task.acceptance_criteria, &result, &authorized)
        .into_iter()
        .filter(|item| targets.contains(&item.criterion_index))
        .collect();
    let mut project = crate::load_project(project_name)?;
    merge_ledger_updates(&mut project, &request.task_id, ledger)?;
    crate::save_project(&project)
}

fn merge_ledger_updates(
    project: &mut project::Project,
    task_id: &str,
    updates: Vec<project::AcceptanceLedgerItem>,
) -> Result<(), String> {
    let task = crate::task_tree::find_task_mut(project, task_id)?
        .ok_or_else(|| format!("任务节点不存在：{}", task_id))?;
    if task.acceptance_ledger.is_empty() {
        task.acceptance_ledger = task
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| project::AcceptanceLedgerItem {
                criterion_index: index as u32 + 1,
                criterion: criterion.clone(),
                ..Default::default()
            })
            .collect();
    }
    for update in updates {
        if let Some(current) = task
            .acceptance_ledger
            .iter_mut()
            .find(|item| item.criterion_index == update.criterion_index)
        {
            *current = update;
        }
    }
    Ok(())
}

fn finish_action(
    project_name: &str,
    request: &ControlActionRequest,
    before_fingerprint: String,
    message: String,
    succeeded: bool,
) -> Result<ControlActionExecutionResult, String> {
    let mut project = crate::load_project(project_name)?;
    if project.task_control.active_action_id != request.action_id {
        return Err("控制动作已被新的项目状态取代".to_string());
    }
    let after_fingerprint = control_fingerprint(&project, &request.task_id)?;
    let made_progress = succeeded && after_fingerprint != before_fingerprint;
    project.task_control.active_action_id.clear();
    project.task_control.active_action_kind.clear();
    project.task_control.active_action_task_id.clear();
    project.task_control.last_completed_action_id = request.action_id.clone();
    project.task_control.last_completed_action_kind = request.action.as_str().to_string();
    project.task_control.last_completed_action_task_id = request.task_id.clone();
    project.task_control.last_action_result = message.clone();
    project.task_control.last_action_made_progress = made_progress;
    project.task_control.last_action_before_fingerprint = before_fingerprint.clone();
    project.task_control.last_action_after_fingerprint = after_fingerprint.clone();
    project.task_control.last_action_at = Some(chrono::Utc::now().to_rfc3339());
    if succeeded
        && !matches!(
            request.action,
            ControlActionKind::Wait | ControlActionKind::Human
        )
    {
        if let Some(state) = project.workflow_state.autopilot_state.as_mut() {
            state.consecutive_no_progress = if made_progress {
                0
            } else {
                state.consecutive_no_progress.saturating_add(1)
            };
        }
    }
    append_control_event(&mut project, request, &message, succeeded);
    project.workflow_state.data_revision = project.workflow_state.data_revision.saturating_add(1);
    crate::save_project(&project)?;
    Ok(ControlActionExecutionResult {
        action_id: request.action_id.clone(),
        action: request.action,
        task_id: request.task_id.clone(),
        lifecycle: if succeeded {
            ControlActionLifecycle::Completed
        } else {
            ControlActionLifecycle::Failed
        },
        idempotent: false,
        queued: false,
        made_progress,
        before_fingerprint,
        after_fingerprint,
        project_revision: project.workflow_state.data_revision,
        tree_revision: project.task_control.tree_revision,
        message,
    })
}

fn append_control_event(
    project: &mut project::Project,
    request: &ControlActionRequest,
    message: &str,
    succeeded: bool,
) {
    let model_call_id = project
        .cost_ledger
        .calls
        .iter()
        .rev()
        .find(|call| call.action_id == request.action_id)
        .map(|call| call.call_id.clone());
    let validator_id = match request.action {
        ControlActionKind::LocalValidate => Some("local_validator_registry".to_string()),
        ControlActionKind::AutomatedValidate => Some("automated_test_runner".to_string()),
        ControlActionKind::TargetedValidate => Some("semantic_review".to_string()),
        ControlActionKind::Human if !request.criterion_indexes.is_empty() => {
            Some("human_boundary_review".to_string())
        }
        _ => None,
    };
    project
        .execution_history
        .push(project::ExecutionHistoryEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: if succeeded { "success" } else { "error" }.to_string(),
            event_type: project::ExecutionEventType::SystemAdvance,
            source: request.source,
            text: message.to_string(),
            milestone_id: (!project.current_milestone_id.is_empty())
                .then(|| project.current_milestone_id.clone()),
            mid_stage_id: (!project.current_mid_stage_id.is_empty())
                .then(|| project.current_mid_stage_id.clone()),
            subtask_id: (!request.task_id.is_empty()).then(|| request.task_id.clone()),
            criterion_index: (request.criterion_indexes.len() == 1)
                .then(|| request.criterion_indexes[0]),
            decision_id: (!request.decision_id.is_empty()).then(|| request.decision_id.clone()),
            action_id: Some(request.action_id.clone()),
            validator_id,
            model_call_id,
        });
    if project.execution_history.len() > project::MAX_EXECUTION_HISTORY {
        let excess = project.execution_history.len() - project::MAX_EXECUTION_HISTORY;
        project.execution_history.drain(0..excess);
    }
}

fn previous_result(
    project: &project::Project,
    request: &ControlActionRequest,
) -> ControlActionExecutionResult {
    ControlActionExecutionResult {
        action_id: request.action_id.clone(),
        action: request.action,
        task_id: request.task_id.clone(),
        lifecycle: ControlActionLifecycle::Completed,
        idempotent: true,
        queued: false,
        made_progress: project.task_control.last_action_made_progress,
        before_fingerprint: project.task_control.last_action_before_fingerprint.clone(),
        after_fingerprint: project.task_control.last_action_after_fingerprint.clone(),
        project_revision: project.workflow_state.data_revision,
        tree_revision: project.task_control.tree_revision,
        message: project.task_control.last_action_result.clone(),
    }
}

fn control_fingerprint(project: &project::Project, task_id: &str) -> Result<String, String> {
    let task = if task_id.is_empty() {
        None
    } else {
        crate::task_tree::find_task(project, task_id)?
    };
    let bytes = serde_json::to_vec(&(
        project.task_control.tree_revision,
        project.workflow_state.current_step.clone(),
        task.map(|task| {
            (
                task.status.clone(),
                task.contract_snapshot.clone(),
                task.acceptance_ledger.clone(),
                task.fact_snapshot.clone(),
                task.child_tasks.len(),
            )
        }),
    ))
    .map_err(|error| format!("控制状态指纹生成失败：{}", error))?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Milestone, MilestoneStatus, StageMode, Subtask};

    fn project_with_task(status: project::SubtaskStatus) -> project::Project {
        let mut project = project::Project::new("executor");
        project.milestones.push(Milestone {
            id: "m".to_string(),
            version: "v0.1".to_string(),
            title: "M".to_string(),
            description: String::new(),
            tech_stack: String::new(),
            status: MilestoneStatus::InProgress,
            mode: StageMode::Quick,
            mid_stages: Vec::new(),
            subtasks: vec![Subtask {
                id: "task".to_string(),
                status,
                acceptance_criteria: vec!["criterion".to_string()],
                ..Default::default()
            }],
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
        });
        project.current_milestone_id = "m".to_string();
        project
    }

    #[test]
    fn execute_rejects_parent_and_non_pending_task() {
        let mut project = project_with_task(project::SubtaskStatus::Pending);
        project.milestones[0].subtasks[0].child_tasks = vec![Subtask {
            id: "child".to_string(),
            ..Default::default()
        }];
        let request = ControlActionRequest {
            action: ControlActionKind::Execute,
            task_id: "task".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("叶子"));
    }

    #[test]
    fn git_confirm_requires_awaiting_leaf() {
        let project = project_with_task(project::SubtaskStatus::Pending);
        let request = ControlActionRequest {
            action: ControlActionKind::GitConfirm,
            task_id: "task".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("待确认"));
    }

    #[test]
    fn phase1_human_action_safety_accepting_deviation_requires_success_and_scope() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.milestones[0].subtasks[0].execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        project.milestones[0].subtasks[0].acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            ..Default::default()
        }];
        let missing_scope = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            reason: "已由用户确认".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &missing_scope)
            .unwrap_err()
            .contains("选择至少一个"));

        let missing_reason = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };
        assert!(validate_request(&project, &missing_reason)
            .unwrap_err()
            .contains("填写依据"));

        project.milestones[0].subtasks[0].execution_result = None;
        let unexecuted = ControlActionRequest {
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            reason: "已由用户确认".to_string(),
            ..Default::default()
        };
        assert!(validate_request(&project, &unexecuted)
            .unwrap_err()
            .contains("没有成功完成"));
    }

    #[test]
    fn phase1_human_action_safety_claimed_dispatch_rejects_newer_revision() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.milestones[0].subtasks[0].execution_result = Some(project::ExecutionResult {
            success: true,
            ..Default::default()
        });
        project.milestones[0].subtasks[0].acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "criterion".to_string(),
            ..Default::default()
        }];
        project.task_control.active_action_id = "action".to_string();
        project.task_control.active_action_kind = "accept_deviation".to_string();
        project.task_control.active_action_task_id = "task".to_string();
        project.workflow_state.data_revision = 8;
        project.task_control.tree_revision = 3;
        let request = ControlActionRequest {
            action_id: "action".to_string(),
            action: ControlActionKind::AcceptDeviation,
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            reason: "known deviation".to_string(),
            ..Default::default()
        };
        validate_claimed_dispatch(&project, &request, 8, 3).unwrap();
        project.workflow_state.data_revision = 9;
        assert!(validate_claimed_dispatch(&project, &request, 8, 3).is_err());
    }

    #[test]
    fn automated_status_updates_only_automated_criteria() {
        let task = Subtask {
            acceptance_criteria: vec!["cargo test 测试通过".to_string()],
            ..Default::default()
        };
        let evidence = crate::automated_validation::AutomatedTestEvidence {
            rendered: None,
            command: "cargo test".to_string(),
            exit_code: Some(0),
            output_summary: "3 passed".to_string(),
            status: project::AutomatedTestStatus::Passed,
        };
        let updates = automated_ledger_updates(&task, &[1], &evidence).unwrap();
        assert_eq!(updates[0].status, project::AcceptanceStatus::Satisfied);
        assert_eq!(updates[0].confidence, 1.0);

        let semantic = Subtask {
            acceptance_criteria: vec!["页面显示测试按钮".to_string()],
            ..Default::default()
        };
        assert!(automated_ledger_updates(&semantic, &[1], &evidence).is_err());
    }

    #[test]
    fn empty_request_filters_targets_to_the_selected_validation_channel() {
        let task = Subtask {
            acceptance_criteria: vec![
                "file exists: `index.html`".to_string(),
                "cargo test 测试通过".to_string(),
                "用户可以完成结账".to_string(),
            ],
            ..Default::default()
        };
        assert_eq!(
            validation_targets_for_mode(
                &task,
                &[],
                crate::validator_contract::VerificationMode::AutomatedTest,
            )
            .unwrap(),
            vec![2]
        );
        assert!(validation_targets_for_mode(
            &task,
            &[3],
            crate::validator_contract::VerificationMode::AutomatedTest,
        )
        .is_err());

        let mut fallback = task;
        fallback.acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            status: project::AcceptanceStatus::Unknown,
            evidence: "local_unprovable:需要语义审查".to_string(),
            ..Default::default()
        }];
        assert_eq!(
            validation_targets_for_mode(
                &fallback,
                &[],
                crate::validator_contract::VerificationMode::SemanticReview,
            )
            .unwrap(),
            vec![1, 3]
        );
    }

    #[test]
    fn human_review_action_accepts_only_human_review_criteria() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        let task = &mut project.milestones[0].subtasks[0];
        task.acceptance_criteria = vec![
            "用户可以完成结账".to_string(),
            "操作员确认真实桌面行为".to_string(),
        ];
        task.acceptance_ledger = task
            .acceptance_criteria
            .iter()
            .enumerate()
            .map(|(index, criterion)| project::AcceptanceLedgerItem {
                criterion_index: index as u32 + 1,
                criterion: criterion.clone(),
                ..Default::default()
            })
            .collect();
        let mut contract = crate::task_contract::compile_subtask(task, None, 0);
        contract.verification_modes = vec![
            crate::validator_contract::VerificationMode::SemanticReview,
            crate::validator_contract::VerificationMode::HumanReview,
        ];
        crate::task_contract::refresh_fingerprint(&mut contract);
        task.contract_snapshot = Some(contract);

        let mut request = ControlActionRequest {
            action: ControlActionKind::Human,
            task_id: "task".to_string(),
            criterion_indexes: vec![2],
            ..Default::default()
        };
        validate_request(&project, &request).unwrap();

        request.criterion_indexes = vec![1];
        assert!(validate_request(&project, &request)
            .unwrap_err()
            .contains("不属于 HumanReview 通道"));
        let task = &project.milestones[0].subtasks[0];
        assert!(validation_targets_for_mode(
            task,
            &[2],
            crate::validator_contract::VerificationMode::SemanticReview,
        )
        .unwrap_err()
        .contains("不属于 SemanticReview 通道"));
    }

    #[test]
    fn human_review_boundary_preserves_ledger_and_uses_human_validator_audit() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        project.workflow_state.autopilot_state = Some(project::AutopilotState::default());
        project.milestones[0].subtasks[0].acceptance_ledger = vec![project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: "操作员确认真实桌面行为".to_string(),
            status: project::AcceptanceStatus::Unknown,
            ..Default::default()
        }];
        let before = project.milestones[0].subtasks[0].acceptance_ledger.clone();
        let request = ControlActionRequest {
            action: ControlActionKind::Human,
            action_id: "human-review-1".to_string(),
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };

        let message = enter_human_boundary(&mut project, &request);
        assert!(message.contains("等待显式人工结论"));
        assert_eq!(project.milestones[0].subtasks[0].acceptance_ledger, before);
        let state = project.workflow_state.autopilot_state.as_ref().unwrap();
        assert_eq!(state.run_status, project::AutopilotRunStatus::ErrorStopped);
        assert_eq!(
            state.recovery_action,
            project::AutopilotRecoveryAction::WaitHumanDecision
        );

        append_control_event(&mut project, &request, &message, true);
        let event = project.execution_history.last().unwrap();
        assert_eq!(event.validator_id.as_deref(), Some("human_boundary_review"));
        assert!(event.model_call_id.is_none());
        assert!(project.cost_ledger.calls.is_empty());
    }

    #[test]
    fn unconfigured_and_unavailable_tests_remain_unknown() {
        let task = Subtask {
            acceptance_criteria: vec!["自动化测试通过".to_string()],
            ..Default::default()
        };
        for status in [
            project::AutomatedTestStatus::NotConfigured,
            project::AutomatedTestStatus::Unavailable,
        ] {
            let evidence = crate::automated_validation::AutomatedTestEvidence {
                rendered: None,
                command: String::new(),
                exit_code: None,
                output_summary: String::new(),
                status,
            };
            let updates = automated_ledger_updates(&task, &[1], &evidence).unwrap();
            assert_eq!(updates[0].status, project::AcceptanceStatus::Unknown);
            assert_eq!(updates[0].confidence, 0.0);
        }
    }

    #[test]
    fn automated_action_audit_uses_test_runner_without_model_call() {
        let mut project = project_with_task(project::SubtaskStatus::AwaitingConfirmation);
        let request = ControlActionRequest {
            action: ControlActionKind::AutomatedValidate,
            action_id: "automated-1".to_string(),
            task_id: "task".to_string(),
            criterion_indexes: vec![1],
            ..Default::default()
        };
        append_control_event(&mut project, &request, "done", true);
        let event = project.execution_history.last().unwrap();
        assert_eq!(event.validator_id.as_deref(), Some("automated_test_runner"));
        assert!(event.model_call_id.is_none());
        assert!(project.cost_ledger.calls.is_empty());
    }
}
