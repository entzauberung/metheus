mod builtin;
mod claude_code;
mod codex;
mod contract;
mod failure_classifier;
mod health;
mod process_runner;
mod service;

pub(crate) use contract::{EngineError, EngineHealth, ExecutionRequest};
pub(crate) use failure_classifier::{blocks_code_recovery, classify_process_failure};
pub(crate) use health::check_engine_health;
pub(crate) use service::{execute, validate_profile};
