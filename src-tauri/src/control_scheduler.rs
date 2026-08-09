use crate::control_action::{ControlAction, ControlActionKind};
use crate::project::{AcceptanceStatus, Subtask, SubtaskStatus};
use crate::task_compiler::{TaskCompileDecisionKind, TaskCompileResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const NO_PROGRESS_STOP_THRESHOLD: u32 = 3;

pub(crate) fn human_review_action_for_cadence(
    cadence: crate::project::HumanReviewCadence,
) -> ControlActionKind {
    match cadence {
        crate::project::HumanReviewCadence::PerTask => ControlActionKind::Human,
        crate::project::HumanReviewCadence::MilestoneBatch => {
            ControlActionKind::ProvisionalValidate
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AcceptanceSummary {
    pub satisfied: u32,
    pub unsatisfied: u32,
    pub unknown: u32,
    pub contradictory: u32,
    pub accepted_deviation: u32,
    pub ai_provisionally_satisfied: u32,
    pub deferred_human_review: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskControlDecision {
    pub decision_id: String,
    pub task_id: String,
    pub contract_fingerprint: String,
    pub facts_fingerprint: String,
    pub acceptance: AcceptanceSummary,
    pub action: ControlAction,
    pub expected_cost: String,
    pub expected_risk: String,
    pub cache_hit: bool,
    pub shadow: bool,
    pub reason: String,
}

pub fn decide_next_action(
    subtask: &Subtask,
    compile_result: &TaskCompileResult,
    facts_fingerprint: &str,
    shadow: bool,
    cadence: crate::project::HumanReviewCadence,
) -> TaskControlDecision {
    let acceptance = summarize(&subtask.acceptance_ledger);
    let (action, reason) = if matches!(
        compile_result.decision.kind,
        TaskCompileDecisionKind::SplitFurther
    ) {
        (
            ControlAction::new(
                ControlActionKind::Split,
                compile_result.decision.reason.clone(),
            ),
            compile_result.decision.reason.clone(),
        )
    } else if subtask.status == SubtaskStatus::Executing {
        (
            ControlAction::new(ControlActionKind::Wait, "当前叶子任务仍在执行"),
            "等待活动执行会话产生新的项目事实".to_string(),
        )
    } else if subtask.status == SubtaskStatus::Pending {
        (
            ControlAction::new(ControlActionKind::Execute, "执行当前原子任务"),
            "任务合同可执行且尚未完成".to_string(),
        )
    } else if acceptance.contradictory > 0 {
        (
            ControlAction::new(
                ControlActionKind::Human,
                "验收结论与阻断证据冲突，需要人工确认",
            ),
            "契约冲突不会自动覆盖".to_string(),
        )
    } else if acceptance.unsatisfied > 0 {
        (
            ControlAction::new(ControlActionKind::Repair, "只修复带有效阻断证据的验收项"),
            "存在明确未满足验收项".to_string(),
        )
    } else if acceptance.unknown > 0 || subtask.acceptance_ledger.is_empty() {
        let local_was_unprovable = subtask.acceptance_ledger.iter().any(|item| {
            item.status == AcceptanceStatus::Unknown
                && item.evidence.starts_with("local_unprovable:")
        });
        let pending =
            crate::acceptance::revalidation_target_indexes(subtask, &[]).unwrap_or_default();
        let modes = pending
            .iter()
            .map(|index| crate::validator_registry::verification_mode_for(subtask, *index))
            .collect::<Vec<_>>();
        let kind = if !local_was_unprovable
            && modes.contains(&crate::validator_contract::VerificationMode::Deterministic)
        {
            ControlActionKind::LocalValidate
        } else if modes.contains(&crate::validator_contract::VerificationMode::AutomatedTest) {
            ControlActionKind::AutomatedValidate
        } else if local_was_unprovable
            || modes.contains(&crate::validator_contract::VerificationMode::SemanticReview)
        {
            ControlActionKind::TargetedValidate
        } else if modes.contains(&crate::validator_contract::VerificationMode::HumanReview) {
            human_review_action_for_cadence(cadence)
        } else {
            ControlActionKind::Human
        };
        (
            ControlAction::new(kind, "仅补充尚未证明的验收项"),
            "证据不足，不重跑已满足项".to_string(),
        )
    } else if subtask.status == SubtaskStatus::AwaitingConfirmation
        && (acceptance.ai_provisionally_satisfied > 0 || acceptance.deferred_human_review > 0)
        && cadence == crate::project::HumanReviewCadence::MilestoneBatch
    {
        (
            ControlAction::new(
                ControlActionKind::GitConfirm,
                "大阶段集中确认项已登记，任务可进入串行确认",
            ),
            "AI 临时结论仅用于阶段内推进，不代表人工确认".to_string(),
        )
    } else if subtask.status == SubtaskStatus::AwaitingConfirmation {
        (
            ControlAction::new(
                ControlActionKind::GitConfirm,
                "验收账本已满足，进入串行确认",
            ),
            "质量条件已满足".to_string(),
        )
    } else if subtask.status == SubtaskStatus::Rejected {
        (
            ControlAction::new(
                ControlActionKind::Human,
                "任务已被驳回且没有可自动修复的阻断证据",
            ),
            "拒绝在缺少明确证据时重复调用编码引擎".to_string(),
        )
    } else {
        (
            ControlAction::new(ControlActionKind::Wait, "等待任务状态变化"),
            "当前状态没有可安全派发的控制动作".to_string(),
        )
    };
    let decision_id = format!("decision-{}", uuid::Uuid::new_v4());
    let cache_hit = subtask.fact_snapshot.as_ref().is_some_and(|facts| {
        !facts.structural_fingerprint.is_empty()
            && facts.structural_fingerprint == facts_fingerprint
    });
    TaskControlDecision {
        decision_id,
        task_id: subtask.id.clone(),
        contract_fingerprint: compile_result.contract.fingerprint.clone(),
        facts_fingerprint: facts_fingerprint.to_string(),
        acceptance,
        action,
        expected_cost: compile_result.contract.budget.level.clone(),
        expected_risk: format!("{:?}", compile_result.contract.risk),
        cache_hit,
        shadow,
        reason,
    }
}

fn summarize(items: &[crate::project::AcceptanceLedgerItem]) -> AcceptanceSummary {
    let mut summary = AcceptanceSummary::default();
    for item in items {
        match item.status {
            AcceptanceStatus::Satisfied => summary.satisfied += 1,
            AcceptanceStatus::Unsatisfied => summary.unsatisfied += 1,
            AcceptanceStatus::Unknown => summary.unknown += 1,
            AcceptanceStatus::Contradictory => summary.contradictory += 1,
            AcceptanceStatus::AcceptedDeviation => summary.accepted_deviation += 1,
            AcceptanceStatus::AiProvisionallySatisfied => summary.ai_provisionally_satisfied += 1,
            AcceptanceStatus::DeferredHumanReview => summary.deferred_human_review += 1,
        }
    }
    summary
}

pub fn decision_fingerprint(decision: &TaskControlDecision) -> String {
    let mut copy = decision.clone();
    copy.decision_id.clear();
    let bytes = serde_json::to_vec(&copy).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub fn should_stop_no_progress(count: u32) -> bool {
    count >= NO_PROGRESS_STOP_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workload() -> crate::project::WorkloadProfile {
        crate::workload_policy::test_profile(crate::project::WorkloadScale::Standard)
    }

    fn compiled_task() -> (Subtask, TaskCompileResult) {
        let mut task = Subtask::default();
        task.id = "t".into();
        task.allowed_file_paths = vec!["index.html".into()];
        task.acceptance_criteria = vec!["DOM 存在 `board` id 节点".into()];
        task.status = SubtaskStatus::AwaitingConfirmation;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        (task, compiled)
    }

    #[test]
    fn unknown_evidence_prefers_local_validation() {
        let (task, compiled) = compiled_task();
        let decision = decide_next_action(
            &task,
            &compiled,
            "facts-1",
            false,
            crate::project::HumanReviewCadence::PerTask,
        );
        assert_eq!(decision.action.kind, ControlActionKind::LocalValidate);
    }

    #[test]
    fn automated_contract_mode_selects_automated_action() {
        let mut task = Subtask::default();
        task.id = "automated".into();
        task.acceptance_criteria = vec!["cargo test 测试通过".into()];
        task.status = SubtaskStatus::AwaitingConfirmation;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        task.contract_snapshot = Some(compiled.contract.clone());
        let decision = decide_next_action(
            &task,
            &compiled,
            "facts",
            false,
            crate::project::HumanReviewCadence::PerTask,
        );
        assert_eq!(decision.action.kind, ControlActionKind::AutomatedValidate);
    }

    #[test]
    fn human_review_contract_mode_enters_human_boundary() {
        let mut task = Subtask::default();
        task.id = "human".into();
        task.acceptance_criteria = vec!["操作员确认真实桌面行为".into()];
        task.status = SubtaskStatus::AwaitingConfirmation;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        let mut contract = compiled.contract.clone();
        contract.verification_modes =
            vec![crate::validator_contract::VerificationMode::HumanReview];
        crate::task_contract::refresh_fingerprint(&mut contract);
        task.contract_snapshot = Some(contract);

        let decision = decide_next_action(
            &task,
            &compiled,
            "facts",
            false,
            crate::project::HumanReviewCadence::PerTask,
        );
        assert_eq!(decision.action.kind, ControlActionKind::Human);

        let batch_decision = decide_next_action(
            &task,
            &compiled,
            "facts",
            false,
            crate::project::HumanReviewCadence::MilestoneBatch,
        );
        assert_eq!(
            batch_decision.action.kind,
            ControlActionKind::ProvisionalValidate
        );
    }

    #[test]
    fn batch_only_defers_human_review_and_preserves_hard_boundaries() {
        let (mut task, _) = compiled_task();
        task.acceptance_ledger = vec![crate::project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: task.acceptance_criteria[0].clone(),
            status: AcceptanceStatus::Contradictory,
            ..Default::default()
        }];
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        assert_eq!(
            decide_next_action(
                &task,
                &compiled,
                "facts",
                false,
                crate::project::HumanReviewCadence::MilestoneBatch,
            )
            .action
            .kind,
            ControlActionKind::Human
        );

        task.acceptance_ledger[0].status = AcceptanceStatus::Unsatisfied;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        assert_eq!(
            decide_next_action(
                &task,
                &compiled,
                "facts",
                false,
                crate::project::HumanReviewCadence::MilestoneBatch,
            )
            .action
            .kind,
            ControlActionKind::Repair
        );

        task.acceptance_ledger[0].status = AcceptanceStatus::Satisfied;
        task.status = SubtaskStatus::Rejected;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        assert_eq!(
            decide_next_action(
                &task,
                &compiled,
                "facts",
                false,
                crate::project::HumanReviewCadence::MilestoneBatch,
            )
            .action
            .kind,
            ControlActionKind::Human
        );
    }

    #[test]
    fn repeated_no_progress_is_bounded() {
        assert!(!should_stop_no_progress(2));
        assert!(should_stop_no_progress(3));
    }

    #[test]
    fn executing_waits_and_satisfied_awaiting_task_confirms() {
        let (mut task, _) = compiled_task();
        task.status = SubtaskStatus::Executing;
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        assert_eq!(
            decide_next_action(
                &task,
                &compiled,
                "facts-1",
                false,
                crate::project::HumanReviewCadence::PerTask,
            )
            .action
            .kind,
            ControlActionKind::Wait
        );

        task.status = SubtaskStatus::AwaitingConfirmation;
        task.acceptance_ledger = vec![crate::project::AcceptanceLedgerItem {
            criterion_index: 1,
            criterion: task.acceptance_criteria[0].clone(),
            status: AcceptanceStatus::Satisfied,
            ..Default::default()
        }];
        let compiled = crate::task_compiler::compile(&task, None, 0, &workload());
        assert_eq!(
            decide_next_action(
                &task,
                &compiled,
                "facts-1",
                false,
                crate::project::HumanReviewCadence::PerTask,
            )
            .action
            .kind,
            ControlActionKind::GitConfirm
        );
    }
}
