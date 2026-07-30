use crate::validator_contract::{
    LocalProofConclusion, ValidatorDescriptor, ValidatorRunMetadata, VerificationMode,
    LOCAL_VALIDATOR_VERSION,
};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct LocalValidationBatch {
    pub criterion_reviews: Vec<crate::project::CriterionReviewResult>,
    pub review_issues: Vec<crate::project::ReviewIssue>,
    pub validator_runs: Vec<ValidatorRunMetadata>,
}

pub fn preferred_mode(criterion: &str) -> VerificationMode {
    if crate::validators::capability(criterion).is_some() {
        VerificationMode::Deterministic
    } else if [
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
    ]
    .iter()
    .any(|token| criterion.to_ascii_lowercase().contains(token))
    {
        VerificationMode::AutomatedTest
    } else {
        VerificationMode::SemanticReview
    }
}

pub fn verification_mode_for(
    task: &crate::project::Subtask,
    criterion_index: u32,
) -> VerificationMode {
    let offset = criterion_index.saturating_sub(1) as usize;
    task.contract_snapshot
        .as_ref()
        .and_then(|contract| contract.verification_modes.get(offset))
        .copied()
        .unwrap_or_else(|| {
            task.acceptance_criteria
                .get(offset)
                .map(|criterion| preferred_mode(criterion))
                .unwrap_or(VerificationMode::SemanticReview)
        })
}

/// Returns None unless every criterion has a deterministic proof strategy.
/// The caller must fall back to its existing semantic-review path on None.
pub fn try_validate_locally(
    project_path: &str,
    criteria: &[String],
    authorized_paths: &[String],
) -> Option<LocalValidationBatch> {
    if criteria.is_empty() || authorized_paths.is_empty() {
        return None;
    }
    let root = std::path::Path::new(project_path);
    let mut criterion_reviews = Vec::new();
    let mut review_issues = Vec::new();
    let mut validator_runs = Vec::new();
    for (offset, criterion) in criteria.iter().enumerate() {
        let proof = crate::validators::validate(root, criterion, authorized_paths)?;
        if !proof.scan_complete || proof.conclusion == LocalProofConclusion::Unprovable {
            return None;
        }
        let evidence_fingerprint = serde_json::to_vec(&(
            proof.validator,
            &proof.proof_scope,
            &proof.evidence_references,
            proof.conclusion,
        ))
        .ok()
        .map(|bytes| format!("sha256:{:x}", Sha256::digest(bytes)))?;
        validator_runs.push(ValidatorRunMetadata {
            validator: proof.validator.to_string(),
            version: LOCAL_VALIDATOR_VERSION.to_string(),
            proof_scope: proof.proof_scope.clone(),
            scan_complete: proof.scan_complete,
            evidence_fingerprint,
        });
        let criterion_index = offset as u32 + 1;
        let conclusion = match proof.conclusion {
            LocalProofConclusion::Satisfied => crate::project::CriterionReviewConclusion::Satisfied,
            LocalProofConclusion::Unsatisfied => {
                crate::project::CriterionReviewConclusion::Unsatisfied
            }
            LocalProofConclusion::Unprovable => return None,
        };
        criterion_reviews.push(crate::project::CriterionReviewResult {
            criterion_index,
            criterion: criterion.clone(),
            conclusion: conclusion.clone(),
            confidence: 1.0,
            evidence_references: proof.evidence_references.clone(),
        });
        if conclusion == crate::project::CriterionReviewConclusion::Unsatisfied {
            review_issues.push(crate::project::ReviewIssue {
                criterion_index: Some(criterion_index),
                criterion: criterion.clone(),
                file: proof
                    .evidence_references
                    .first()
                    .map(|reference| reference.file.clone())
                    .unwrap_or_default(),
                expected: proof.expected,
                actual: proof.actual,
                suggested_change: proof.suggested_change,
                confidence: 1.0,
                severity: Some(crate::project::ReviewIssueSeverity::Blocking),
                evidence_references: proof.evidence_references,
            });
        }
    }
    Some(LocalValidationBatch {
        criterion_reviews,
        review_issues,
        validator_runs,
    })
}

pub fn validators_for(criterion: &str) -> Vec<ValidatorDescriptor> {
    match preferred_mode(criterion) {
        VerificationMode::Deterministic => {
            let (name, scope) = crate::validators::capability(criterion)
                .unwrap_or(("local_proof", "conservative local proof"));
            vec![ValidatorDescriptor::local(name, scope)]
        }
        VerificationMode::AutomatedTest => vec![ValidatorDescriptor {
            name: "automated_test_runner".to_string(),
            mode: VerificationMode::AutomatedTest,
            risk: crate::validator_contract::ValidatorRisk::Medium,
            cost: crate::validator_contract::ValidatorCost::Local,
            deterministic: true,
            proof_scope: "declared test command output".to_string(),
            version: LOCAL_VALIDATOR_VERSION.to_string(),
            requires_complete_scan: true,
            technology: "generic".to_string(),
        }],
        VerificationMode::SemanticReview => vec![ValidatorDescriptor {
            name: "targeted_semantic_review".to_string(),
            mode: VerificationMode::SemanticReview,
            risk: crate::validator_contract::ValidatorRisk::Medium,
            cost: crate::validator_contract::ValidatorCost::Model,
            deterministic: false,
            proof_scope: "task-specific semantic acceptance criterion".to_string(),
            version: "semantic-review-v1".to_string(),
            requires_complete_scan: false,
            technology: "generic".to_string(),
        }],
        VerificationMode::HumanReview => vec![ValidatorDescriptor {
            name: "human_boundary_review".to_string(),
            mode: VerificationMode::HumanReview,
            risk: crate::validator_contract::ValidatorRisk::High,
            cost: crate::validator_contract::ValidatorCost::Free,
            deterministic: false,
            proof_scope: "explicit user decision".to_string(),
            version: "human-review-v1".to_string(),
            requires_complete_scan: false,
            technology: "generic".to_string(),
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_test_button_is_semantic_not_an_automated_test_command() {
        assert_eq!(
            preferred_mode("页面显示测试按钮"),
            VerificationMode::SemanticReview
        );
        assert_eq!(
            preferred_mode("cargo test 测试通过"),
            VerificationMode::AutomatedTest
        );
    }
    use crate::validator_contract::ValidatorCost;

    #[test]
    fn local_fact_checks_are_selected_first() {
        let validators = validators_for("不得存在硬编码颜色");
        assert_eq!(validators[0].mode, VerificationMode::Deterministic);
        assert_eq!(validators[0].cost, ValidatorCost::Free);
    }

    #[test]
    fn semantic_criteria_are_the_only_model_fallback() {
        let validators = validators_for("the user can complete checkout");
        assert_eq!(validators[0].mode, VerificationMode::SemanticReview);
    }

    #[test]
    fn local_validation_requires_concrete_proof_tokens() {
        let root =
            std::env::temp_dir().join(format!("metheus-local-validator-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("index.html"), "<div id=\"board\"></div>").unwrap();
        let batch = try_validate_locally(
            root.to_str().unwrap(),
            &["DOM 包含 `board` 节点".to_string()],
            &["index.html".to_string()],
        )
        .unwrap();
        assert!(batch.review_issues.is_empty());
        assert_eq!(batch.criterion_reviews.len(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn function_name_occurrence_does_not_prove_behavior() {
        let root =
            std::env::temp_dir().join(format!("metheus-local-validator-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("app.js"), "// submitOrder is mentioned only\n").unwrap();
        assert!(try_validate_locally(
            root.to_str().unwrap(),
            &["函数 `submitOrder` 能正确完成订单提交流程".to_string()],
            &["app.js".to_string()],
        )
        .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn css_scan_ignores_comments_and_reports_real_declarations() {
        let root =
            std::env::temp_dir().join(format!("metheus-css-validator-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("app.css"),
            "/* color: #fff */\n:root { --surface: #fff; }\n.panel { color: var(--surface); }",
        )
        .unwrap();
        let clean = try_validate_locally(
            root.to_str().unwrap(),
            &["CSS 中不得存在硬编码颜色".to_string()],
            &["app.css".to_string()],
        )
        .unwrap();
        assert_eq!(
            clean.criterion_reviews[0].conclusion,
            crate::project::CriterionReviewConclusion::Satisfied
        );

        std::fs::write(root.join("app.css"), ".panel { color: #fff; }").unwrap();
        let failed = try_validate_locally(
            root.to_str().unwrap(),
            &["CSS 中不得存在硬编码颜色".to_string()],
            &["app.css".to_string()],
        )
        .unwrap();
        assert_eq!(
            failed.criterion_reviews[0].conclusion,
            crate::project::CriterionReviewConclusion::Unsatisfied
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
