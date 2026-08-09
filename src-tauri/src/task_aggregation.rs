use crate::project::{AcceptanceLedgerItem, AcceptanceStatus, Project, Subtask, SubtaskStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TaskAggregationOutcome {
    pub completed_task_id: String,
    pub updated_parent_ids: Vec<String>,
    pub next_leaf_id: Option<String>,
    pub contract_conflict: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct ParentAggregation {
    status: SubtaskStatus,
    ledger: Vec<AcceptanceLedgerItem>,
    source_task_ids: Vec<String>,
    deviation_criteria: Vec<u32>,
    reason: String,
    conflict: bool,
}

pub fn aggregate_ancestors(
    project: &mut Project,
    completed_task_id: &str,
) -> Result<TaskAggregationOutcome, String> {
    let address = crate::task_tree::locate_task(project, completed_task_id)?
        .ok_or_else(|| format!("聚合源任务不存在：{}", completed_task_id))?;
    let completed = crate::task_tree::find_task(project, completed_task_id)?
        .ok_or_else(|| format!("聚合源任务不存在：{}", completed_task_id))?;
    if !crate::task_tree::is_terminal(&completed.status) {
        return Err("只有进入终态的叶子任务才能触发父节点聚合".to_string());
    }
    validate_terminal_source(project, completed)?;

    let mut outcome = TaskAggregationOutcome {
        completed_task_id: completed_task_id.to_string(),
        ..Default::default()
    };
    for parent_id in address.ancestor_task_ids.iter().rev() {
        let parent = crate::task_tree::find_task(project, parent_id)?
            .ok_or_else(|| format!("聚合父任务不存在：{}", parent_id))?
            .clone();
        let aggregation = aggregate_parent(
            &parent,
            project.human_review_cadence == crate::project::HumanReviewCadence::MilestoneBatch,
        );
        let parent_mut = crate::task_tree::find_task_mut(project, parent_id)?
            .ok_or_else(|| format!("聚合父任务不存在：{}", parent_id))?;
        parent_mut.status = aggregation.status;
        parent_mut.acceptance_ledger = aggregation.ledger;
        parent_mut.aggregated_at = Some(chrono::Utc::now().to_rfc3339());
        parent_mut.aggregation_source_task_ids = aggregation.source_task_ids;
        parent_mut.affected_deviation_criteria = aggregation.deviation_criteria;
        parent_mut.aggregation_reason = aggregation.reason;
        outcome.contract_conflict |= aggregation.conflict;
        outcome.updated_parent_ids.push(parent_id.clone());
    }
    outcome.next_leaf_id =
        crate::task_tree::select_current_leaf(project)?.map(|address| address.task_id);
    Ok(outcome)
}

fn validate_terminal_source(project: &Project, task: &Subtask) -> Result<(), String> {
    match task.status {
        SubtaskStatus::AcceptedDeviation => {
            crate::human_action_policy::validate_recorded_human_acceptance(project, task)
                .map_err(|reason| format!("接受偏差任务审计无效，禁止父节点聚合：{}", reason))?;
        }
        SubtaskStatus::Skipped => {
            let verification = task
                .human_verification
                .as_ref()
                .filter(|verification| {
                    verification.resolution == crate::project::HumanResolution::SkipTask
                })
                .ok_or_else(|| "跳过任务缺少后端人工动作审计，禁止父节点聚合".to_string())?;
            if verification.action_source.is_empty() || verification.dependency_check.is_empty() {
                return Err("跳过任务的依赖审计不完整，禁止父节点聚合".to_string());
            }
        }
        _ => {}
    }
    Ok(())
}

fn aggregate_parent(parent: &Subtask, allow_deferred: bool) -> ParentAggregation {
    let now = chrono::Utc::now().to_rfc3339();
    let all_children_terminal = !parent.child_tasks.is_empty()
        && parent
            .child_tasks
            .iter()
            .all(|child| crate::task_tree::is_terminal(&child.status));
    let mut ledger = parent
        .acceptance_criteria
        .iter()
        .enumerate()
        .map(|(index, criterion)| AcceptanceLedgerItem {
            criterion_index: index as u32 + 1,
            criterion: criterion.clone(),
            updated_at: now.clone(),
            ..Default::default()
        })
        .collect::<Vec<_>>();

    for item in &mut ledger {
        let proofs = parent
            .child_tasks
            .iter()
            .filter_map(|child| {
                child_proof_for_parent(child, item.criterion_index, &item.criterion)
            })
            .collect::<Vec<_>>();
        if proofs.is_empty() {
            continue;
        }
        item.status = aggregate_proof_status(&proofs);
        item.evidence = proofs
            .iter()
            .map(|proof| format!("{}：{}", proof.source_task_id, proof.evidence))
            .collect::<Vec<_>>()
            .join("；");
        item.evidence_references = proofs
            .iter()
            .flat_map(|proof| proof.references.iter().cloned())
            .collect();
        item.confidence = proofs
            .iter()
            .map(|proof| proof.confidence)
            .fold(1.0_f64, f64::min);
    }

    let conflict = ledger
        .iter()
        .any(|item| item.status == AcceptanceStatus::Contradictory);
    let all_criteria_proven = ledger.iter().all(|item| {
        matches!(
            item.status,
            AcceptanceStatus::Satisfied
                | AcceptanceStatus::AcceptedDeviation
                | AcceptanceStatus::AiProvisionallySatisfied
                | AcceptanceStatus::DeferredHumanReview
        ) && (allow_deferred
            || !matches!(
                item.status,
                AcceptanceStatus::AiProvisionallySatisfied | AcceptanceStatus::DeferredHumanReview
            ))
    });
    let has_deviation = ledger
        .iter()
        .any(|item| item.status == AcceptanceStatus::AcceptedDeviation)
        || parent.child_tasks.iter().any(|child| {
            matches!(
                child.status,
                SubtaskStatus::AcceptedDeviation | SubtaskStatus::Skipped
            )
        });
    let status = if all_children_terminal && all_criteria_proven && !conflict {
        if has_deviation {
            SubtaskStatus::AcceptedDeviation
        } else {
            SubtaskStatus::Passed
        }
    } else {
        SubtaskStatus::Pending
    };
    let source_task_ids = parent
        .child_tasks
        .iter()
        .filter(|child| crate::task_tree::is_terminal(&child.status))
        .map(|child| child.id.clone())
        .collect::<Vec<_>>();
    let deviation_criteria = ledger
        .iter()
        .filter(|item| item.status == AcceptanceStatus::AcceptedDeviation)
        .map(|item| item.criterion_index)
        .collect::<Vec<_>>();
    ParentAggregation {
        status,
        ledger,
        source_task_ids,
        deviation_criteria,
        reason: if conflict {
            "子任务证据发生契约冲突，等待人工处理".to_string()
        } else if !all_children_terminal {
            "仍有未完成子任务".to_string()
        } else if !all_criteria_proven {
            "子任务已结束，但父验收证据尚未完整映射".to_string()
        } else if has_deviation {
            "全部必需子任务完成，父任务包含范围化偏差".to_string()
        } else {
            "全部必需子任务及父验收证据已完成".to_string()
        },
        conflict,
    }
}

#[derive(Debug)]
struct ChildProof {
    source_task_id: String,
    status: AcceptanceStatus,
    evidence: String,
    references: Vec<crate::project::ReviewEvidenceReference>,
    confidence: f64,
}

fn child_proof_for_parent(
    child: &Subtask,
    parent_index: u32,
    parent_criterion: &str,
) -> Option<ChildProof> {
    let local_index = child
        .parent_criterion_indexes
        .iter()
        .position(|index| *index == parent_index)
        .map(|index| index as u32 + 1)
        .or_else(|| {
            child
                .acceptance_criteria
                .iter()
                .position(|criterion| criterion == parent_criterion)
                .map(|index| index as u32 + 1)
        })?;
    if child.status == SubtaskStatus::Skipped {
        return Some(ChildProof {
            source_task_id: child.id.clone(),
            status: AcceptanceStatus::AcceptedDeviation,
            evidence: child
                .human_verification
                .as_ref()
                .map(|verification| verification.verification_reason.clone())
                .filter(|reason| !reason.is_empty())
                .unwrap_or_else(|| "子任务经依赖检查后跳过".to_string()),
            references: Vec::new(),
            confidence: 1.0,
        });
    }
    let ledger = child
        .acceptance_ledger
        .iter()
        .find(|item| item.criterion_index == local_index)?;
    Some(ChildProof {
        source_task_id: child.id.clone(),
        status: ledger.status.clone(),
        evidence: ledger.evidence.clone(),
        references: ledger.evidence_references.clone(),
        confidence: ledger.confidence,
    })
}

fn aggregate_proof_status(proofs: &[ChildProof]) -> AcceptanceStatus {
    if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::Contradictory)
    {
        AcceptanceStatus::Contradictory
    } else if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::Unsatisfied)
    {
        AcceptanceStatus::Unsatisfied
    } else if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::Unknown)
    {
        AcceptanceStatus::Unknown
    } else if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::DeferredHumanReview)
    {
        AcceptanceStatus::DeferredHumanReview
    } else if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::AiProvisionallySatisfied)
    {
        AcceptanceStatus::AiProvisionallySatisfied
    } else if proofs
        .iter()
        .any(|proof| proof.status == AcceptanceStatus::AcceptedDeviation)
    {
        AcceptanceStatus::AcceptedDeviation
    } else {
        AcceptanceStatus::Satisfied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Milestone, MilestoneStatus, StageMode};

    fn proven_child(id: &str, parent_index: u32, status: AcceptanceStatus) -> Subtask {
        Subtask {
            id: id.to_string(),
            title: id.to_string(),
            status: if status == AcceptanceStatus::AcceptedDeviation {
                SubtaskStatus::AcceptedDeviation
            } else {
                SubtaskStatus::Passed
            },
            acceptance_criteria: vec![format!("criterion {}", parent_index)],
            acceptance_ledger: vec![AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: format!("criterion {}", parent_index),
                status,
                evidence: format!("evidence-{id}"),
                confidence: 1.0,
                ..Default::default()
            }],
            parent_criterion_indexes: vec![parent_index],
            ..Default::default()
        }
    }

    fn project_with_parent(parent: Subtask) -> Project {
        let mut project = Project::new("aggregation");
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
            subtasks: vec![parent],
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
    fn aggregates_real_child_evidence_without_git_side_effects() {
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string(), "criterion 2".to_string()],
            child_tasks: vec![
                proven_child("one", 1, AcceptanceStatus::Satisfied),
                proven_child("two", 2, AcceptanceStatus::Satisfied),
            ],
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        let outcome = aggregate_ancestors(&mut project, "two").unwrap();
        let parent = &project.milestones[0].subtasks[0];
        assert_eq!(parent.status, SubtaskStatus::Passed);
        assert!(parent.auto_tag.is_none());
        assert_eq!(parent.acceptance_ledger.len(), 2);
        assert_eq!(outcome.updated_parent_ids, vec!["parent"]);
    }

    #[test]
    fn phase1_human_action_safety_illegal_deviation_cannot_complete_parent() {
        let child = proven_child("deviation", 1, AcceptanceStatus::AcceptedDeviation);
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string()],
            child_tasks: vec![child],
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        assert!(aggregate_ancestors(&mut project, "deviation")
            .unwrap_err()
            .contains("审计"));
        assert_eq!(
            project.milestones[0].subtasks[0].status,
            SubtaskStatus::Pending
        );
    }

    #[test]
    fn unknown_child_evidence_keeps_parent_incomplete() {
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string(), "criterion 2".to_string()],
            child_tasks: vec![
                proven_child("one", 1, AcceptanceStatus::Satisfied),
                proven_child("two", 2, AcceptanceStatus::Unknown),
            ],
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        aggregate_ancestors(&mut project, "two").unwrap();
        assert_eq!(
            project.milestones[0].subtasks[0].status,
            SubtaskStatus::Pending
        );
    }

    #[test]
    fn batch_parent_and_runtime_snapshot_preserve_temporary_review_states() {
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string(), "criterion 2".to_string()],
            child_tasks: vec![
                proven_child("one", 1, AcceptanceStatus::AiProvisionallySatisfied),
                proven_child("two", 2, AcceptanceStatus::DeferredHumanReview),
            ],
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        let outcome = aggregate_ancestors(&mut project, "two").unwrap();
        assert!(!outcome.contract_conflict);
        let parent = &project.milestones[0].subtasks[0];
        assert_eq!(parent.status, SubtaskStatus::Passed);
        assert_eq!(
            parent.acceptance_ledger[0].status,
            AcceptanceStatus::AiProvisionallySatisfied
        );
        assert_eq!(
            parent.acceptance_ledger[1].status,
            AcceptanceStatus::DeferredHumanReview
        );

        project.milestones[0].human_review_items = vec![crate::project::MilestoneHumanReviewItem {
            ai_status: AcceptanceStatus::AiProvisionallySatisfied,
            human_decision: crate::project::MilestoneHumanDecision::Pending,
            ..Default::default()
        }];
        let snapshot = crate::runtime_snapshot::compose_runtime_snapshot(
            project,
            None,
            crate::project_state_bus::ProjectStateSubscription {
                subscription_id: String::new(),
                process_start_id: "aggregation-test".to_string(),
                event_sequence: 1,
            },
        );
        let snapshot_parent = &snapshot.project.milestones[0].subtasks[0];
        assert_eq!(
            snapshot_parent.acceptance_ledger[0].status,
            AcceptanceStatus::AiProvisionallySatisfied
        );
        assert_eq!(
            snapshot_parent.acceptance_ledger[1].status,
            AcceptanceStatus::DeferredHumanReview
        );
        assert_eq!(
            snapshot.project.milestones[0].human_review_items[0].human_decision,
            crate::project::MilestoneHumanDecision::Pending
        );
    }

    #[test]
    fn contradictory_child_evidence_remains_a_batch_contract_conflict() {
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string()],
            child_tasks: vec![
                proven_child("one", 1, AcceptanceStatus::AiProvisionallySatisfied),
                proven_child("two", 1, AcceptanceStatus::Contradictory),
            ],
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        let outcome = aggregate_ancestors(&mut project, "two").unwrap();
        let parent = &project.milestones[0].subtasks[0];
        assert!(outcome.contract_conflict);
        assert_eq!(parent.status, SubtaskStatus::Pending);
        assert_eq!(
            parent.acceptance_ledger[0].status,
            AcceptanceStatus::Contradictory
        );
    }

    #[test]
    fn nested_parents_aggregate_from_leaf_to_root() {
        let inner = Subtask {
            id: "inner".to_string(),
            title: "Inner".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string()],
            parent_criterion_indexes: vec![1],
            child_tasks: vec![proven_child("leaf", 1, AcceptanceStatus::Satisfied)],
            ..Default::default()
        };
        let root = Subtask {
            id: "root".to_string(),
            title: "Root".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string()],
            child_tasks: vec![inner],
            ..Default::default()
        };
        let mut project = project_with_parent(root);
        let outcome = aggregate_ancestors(&mut project, "leaf").unwrap();
        assert_eq!(outcome.updated_parent_ids, vec!["inner", "root"]);
        assert_eq!(
            project.milestones[0].subtasks[0].status,
            SubtaskStatus::Passed
        );
    }

    #[test]
    fn phase1_runtime_contract_two_leaf_closeout_aggregates_parent_and_advances() {
        let mut first = Subtask {
            id: "leaf-one".to_string(),
            title: "Leaf one".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string()],
            parent_criterion_indexes: vec![1],
            ..Default::default()
        };
        let mut second = Subtask {
            id: "leaf-two".to_string(),
            title: "Leaf two".to_string(),
            acceptance_criteria: vec!["criterion 2".to_string()],
            parent_criterion_indexes: vec![2],
            depends_on: vec!["leaf-one".to_string()],
            ..Default::default()
        };
        let parent = Subtask {
            id: "parent".to_string(),
            title: "Parent".to_string(),
            acceptance_criteria: vec!["criterion 1".to_string(), "criterion 2".to_string()],
            child_tasks: vec![first.clone(), second.clone()],
            ..Default::default()
        };
        let next = Subtask {
            id: "next-top-level".to_string(),
            title: "Next".to_string(),
            ..Default::default()
        };
        let mut project = project_with_parent(parent);
        project.milestones[0].subtasks.push(next);

        assert_eq!(
            crate::task_tree::select_current_leaf(&project)
                .unwrap()
                .unwrap()
                .task_id,
            "leaf-one"
        );

        first.status = SubtaskStatus::Passed;
        first.auto_tag = Some("task-leaf-one".to_string());
        first.acceptance_ledger =
            proven_child("leaf-one", 1, AcceptanceStatus::Satisfied).acceptance_ledger;
        *crate::task_tree::find_task_mut(&mut project, "leaf-one")
            .unwrap()
            .unwrap() = first;
        let first_outcome = aggregate_ancestors(&mut project, "leaf-one").unwrap();
        assert_eq!(first_outcome.next_leaf_id.as_deref(), Some("leaf-two"));
        assert_eq!(
            crate::task_tree::find_task(&project, "parent")
                .unwrap()
                .unwrap()
                .status,
            SubtaskStatus::Pending
        );

        second.status = SubtaskStatus::Passed;
        second.auto_tag = Some("task-leaf-two".to_string());
        second.acceptance_ledger =
            proven_child("leaf-two", 2, AcceptanceStatus::Satisfied).acceptance_ledger;
        *crate::task_tree::find_task_mut(&mut project, "leaf-two")
            .unwrap()
            .unwrap() = second;
        let final_outcome = aggregate_ancestors(&mut project, "leaf-two").unwrap();
        assert_eq!(
            final_outcome.next_leaf_id.as_deref(),
            Some("next-top-level")
        );
        let parent = crate::task_tree::find_task(&project, "parent")
            .unwrap()
            .unwrap();
        assert_eq!(parent.status, SubtaskStatus::Passed);
        assert_eq!(parent.acceptance_ledger.len(), 2);
        assert!(parent.auto_tag.is_none());
    }
}
