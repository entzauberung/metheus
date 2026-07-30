use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControlActionKind {
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
}

impl Default for ControlActionKind {
    fn default() -> Self {
        Self::Wait
    }
}

impl ControlActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Split => "split",
            Self::Execute => "execute",
            Self::LocalValidate => "local_validate",
            Self::AutomatedValidate => "automated_validate",
            Self::TargetedValidate => "targeted_validate",
            Self::Repair => "repair",
            Self::Recompile => "recompile",
            Self::AcceptDeviation => "accept_deviation",
            Self::GitConfirm => "git_confirm",
            Self::Wait => "wait",
            Self::Human => "human",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ControlActionLifecycle {
    #[default]
    Claimed,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ControlAction {
    pub kind: ControlActionKind,
    pub priority: u8,
    pub risk: String,
    pub reason: String,
    pub retryable: bool,
}

impl ControlAction {
    pub fn new(kind: ControlActionKind, reason: impl Into<String>) -> Self {
        Self {
            kind,
            priority: 50,
            risk: "medium".to_string(),
            reason: reason.into(),
            retryable: false,
        }
    }
}
