use serde::{Deserialize, Serialize};

pub const LOCAL_VALIDATOR_VERSION: &str = "local-proof-v1";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum VerificationMode {
    #[default]
    Deterministic,
    AutomatedTest,
    SemanticReview,
    HumanReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ValidatorRisk {
    #[default]
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ValidatorCost {
    #[default]
    Free,
    Local,
    Model,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorDescriptor {
    pub name: String,
    pub mode: VerificationMode,
    pub risk: ValidatorRisk,
    pub cost: ValidatorCost,
    pub deterministic: bool,
    pub proof_scope: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub requires_complete_scan: bool,
    #[serde(default)]
    pub technology: String,
}

impl ValidatorDescriptor {
    pub fn local(name: &str, scope: &str) -> Self {
        Self {
            name: name.to_string(),
            mode: VerificationMode::Deterministic,
            risk: ValidatorRisk::Low,
            cost: ValidatorCost::Free,
            deterministic: true,
            proof_scope: scope.to_string(),
            version: LOCAL_VALIDATOR_VERSION.to_string(),
            requires_complete_scan: true,
            technology: "generic".to_string(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LocalProofConclusion {
    Satisfied,
    Unsatisfied,
    Unprovable,
}

#[derive(Debug, Clone)]
pub struct LocalProof {
    pub validator: &'static str,
    pub conclusion: LocalProofConclusion,
    pub scan_complete: bool,
    pub proof_scope: String,
    pub evidence_references: Vec<crate::project::ReviewEvidenceReference>,
    pub expected: String,
    pub actual: String,
    pub suggested_change: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ValidatorRunMetadata {
    pub validator: String,
    pub version: String,
    pub proof_scope: String,
    pub scan_complete: bool,
    pub evidence_fingerprint: String,
}
