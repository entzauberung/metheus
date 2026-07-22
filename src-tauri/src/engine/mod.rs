mod builtin;
mod claude_code;
mod codex;
mod contract;
mod health;
mod process_runner;
mod service;

pub(crate) use contract::{EngineError, EngineHealth, ExecutionRequest};
pub(crate) use health::check_engine_health;
pub(crate) use service::{execute, validate_profile};
