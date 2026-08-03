use serde::{Deserialize, Serialize};

/// How an acceptance statement can be proven. This is deliberately separate
/// from the validator implementation so planning can preserve the boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum Provability {
    Deterministic,
    AutomatedTest,
    SemanticReview,
    #[default]
    HumanReview,
    Unprovable,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ProvabilitySource {
    PlanningExplicit,
    #[default]
    SystemInferred,
    HumanCorrected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct AcceptanceCriterion {
    pub text: String,
    #[serde(default)]
    pub provability: Provability,
    #[serde(default)]
    pub provability_source: ProvabilitySource,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum EvidenceSourceType {
    LocalScan,
    AutomatedTestOutput,
    CodeSnippet,
    ExpandedCodeSnippet,
    RuntimeOrHuman,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct EvidenceSourceFingerprint {
    pub fingerprint: String,
    #[serde(default)]
    pub source_types: Vec<EvidenceSourceType>,
    #[serde(default)]
    pub covered_files: Vec<String>,
    #[serde(default)]
    pub validator_type: String,
}

impl Provability {
    pub fn verification_mode(self) -> crate::validator_contract::VerificationMode {
        match self {
            Self::Deterministic => crate::validator_contract::VerificationMode::Deterministic,
            Self::AutomatedTest => crate::validator_contract::VerificationMode::AutomatedTest,
            Self::SemanticReview => crate::validator_contract::VerificationMode::SemanticReview,
            Self::HumanReview | Self::Unprovable => {
                crate::validator_contract::VerificationMode::HumanReview
            }
        }
    }

    fn conservative_rank(self) -> u8 {
        match self {
            Self::Deterministic => 0,
            Self::AutomatedTest => 1,
            Self::SemanticReview => 2,
            Self::HumanReview => 3,
            Self::Unprovable => 4,
        }
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

/// Conservative local inference used for legacy data and as a check on model
/// labels. Unknown statements go to a human, never to an optimistic scanner.
pub fn infer_provability(criterion: &str) -> Provability {
    let normalized = criterion.trim().to_ascii_lowercase();
    if normalized.is_empty()
        || contains_any(
            &normalized,
            &[
                "无法证明",
                "不可验证",
                "绝对完美",
                "100% 无缺陷",
                "always flawless",
            ],
        )
    {
        return Provability::Unprovable;
    }
    if crate::validators::capability(criterion).is_some() {
        return Provability::Deterministic;
    }
    if contains_any(
        &normalized,
        &[
            "automated test",
            "test suite",
            "tests pass",
            "run tests",
            "npm test",
            "pnpm test",
            "yarn test",
            "cargo test",
            "go test",
            "pytest",
            "ctest",
            "mvn test",
            "gradle test",
            "lint passes",
            "build succeeds",
            "compile succeeds",
            "自动化测试",
            "测试命令",
            "测试套件",
            "测试通过",
            "构建通过",
            "编译通过",
            "lint 通过",
        ],
    ) {
        return Provability::AutomatedTest;
    }
    if contains_any(
        &normalized,
        &[
            "视觉",
            "样式一致",
            "保持一致",
            "与打磨前一致",
            "体验",
            "美观",
            "主观",
            "手感",
            "观感",
            "visual",
            "look and feel",
            "pixel perfect",
            "user experience",
            "operator confirms",
            "人工确认",
            "真实桌面",
        ],
    ) {
        return Provability::HumanReview;
    }
    if contains_any(
        &normalized,
        &[
            "逻辑",
            "正确",
            "返回",
            "处理",
            "调用",
            "兼容",
            "错误",
            "数据",
            "流程",
            "能够",
            "可以",
            "logic",
            "returns",
            "handles",
            "calls",
            "compatible",
            "error",
            "data",
            "flow",
            " can ",
        ],
    ) {
        return Provability::SemanticReview;
    }
    Provability::HumanReview
}

pub fn criterion_provability(
    criteria: &[String],
    metadata: &[AcceptanceCriterion],
    criterion_index: u32,
) -> Option<Provability> {
    let offset = criterion_index.checked_sub(1)? as usize;
    let text = criteria.get(offset)?;
    metadata
        .get(offset)
        .filter(|item| item.text == *text)
        .map(|item| item.provability)
}

/// Align declared metadata with the authoritative string array. Model labels
/// are checked against local inference and the more conservative result wins;
/// an explicit human correction remains authoritative.
pub fn normalize_metadata(
    criteria: &[String],
    declared: &[AcceptanceCriterion],
) -> Vec<AcceptanceCriterion> {
    criteria
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let inferred = infer_provability(text);
            let supplied = declared.get(index).filter(|item| item.text == *text);
            let (mut provability, mut source) = match supplied {
                Some(item) if item.provability_source == ProvabilitySource::HumanCorrected => {
                    (item.provability, item.provability_source)
                }
                Some(item)
                    if item.provability.conservative_rank() >= inferred.conservative_rank() =>
                {
                    (item.provability, item.provability_source)
                }
                _ => (inferred, ProvabilitySource::SystemInferred),
            };
            if provability == Provability::Unprovable {
                provability = Provability::HumanReview;
                source = ProvabilitySource::SystemInferred;
            }
            AcceptanceCriterion {
                text: text.clone(),
                provability,
                provability_source: source,
            }
        })
        .collect()
}

fn migrate_task(task: &mut crate::project::Subtask) -> bool {
    let normalized = normalize_metadata(&task.acceptance_criteria, &task.acceptance_criteria_meta);
    let mut changed = normalized != task.acceptance_criteria_meta;
    task.acceptance_criteria_meta = normalized;
    for child in &mut task.child_tasks {
        changed |= migrate_task(child);
    }
    changed
}

/// Purely local, idempotent migration. It only adds proof metadata and never
/// changes criterion text or an existing ledger conclusion.
pub fn migrate_project_metadata(project: &mut crate::project::Project) -> bool {
    let mut changed = false;
    for milestone in &mut project.milestones {
        for task in &mut milestone.subtasks {
            changed |= migrate_task(task);
        }
        for stage in &mut milestone.mid_stages {
            for task in &mut stage.subtasks {
                changed |= migrate_task(task);
            }
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provability_closeout_visual_statements_are_human_review() {
        assert_eq!(
            infer_provability("视觉表现与打磨前一致"),
            Provability::HumanReview
        );
    }

    #[test]
    fn provability_closeout_unknown_inference_is_conservative() {
        assert_eq!(
            infer_provability("令人满意的最终结果"),
            Provability::HumanReview
        );
    }

    #[test]
    fn provability_closeout_unprovable_is_downgraded_to_human_boundary() {
        let criteria = vec!["保证绝对完美".to_string()];
        let normalized = normalize_metadata(&criteria, &[]);
        assert_eq!(normalized[0].provability, Provability::HumanReview);
        assert_eq!(
            normalized[0].provability_source,
            ProvabilitySource::SystemInferred
        );
    }

    #[test]
    fn provability_closeout_legacy_migration_preserves_ledger_status() {
        let mut task = crate::project::Subtask {
            acceptance_criteria: vec!["视觉表现与打磨前一致".to_string()],
            acceptance_ledger: vec![crate::project::AcceptanceLedgerItem {
                criterion_index: 1,
                criterion: "视觉表现与打磨前一致".to_string(),
                status: crate::project::AcceptanceStatus::Satisfied,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(migrate_task(&mut task));
        assert_eq!(task.acceptance_criteria_meta.len(), 1);
        assert_eq!(
            task.acceptance_criteria_meta[0].provability,
            Provability::HumanReview
        );
        assert_eq!(
            task.acceptance_ledger[0].status,
            crate::project::AcceptanceStatus::Satisfied
        );
        assert!(!migrate_task(&mut task));
    }

    #[test]
    fn provability_closeout_legacy_subtask_json_without_metadata_still_loads() {
        let mut value = serde_json::to_value(crate::project::Subtask {
            acceptance_criteria: vec!["视觉表现与打磨前一致".to_string()],
            ..Default::default()
        })
        .unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("acceptance_criteria_meta");
        let restored: crate::project::Subtask = serde_json::from_value(value).unwrap();
        assert!(restored.acceptance_criteria_meta.is_empty());
        assert_eq!(restored.acceptance_criteria.len(), 1);
    }
}
