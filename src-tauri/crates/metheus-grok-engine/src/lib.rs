mod config;
mod error;
mod event_bridge;
mod model_adapter;
mod runtime;

pub use config::{
    GrokBuildApiBackend, GrokBuildExecutionConfig, GrokBuildExecutionRequest,
    GrokBuildExecutionResult,
};
pub use error::{GrokBuildRuntimeError, GrokBuildRuntimeErrorKind};
pub use event_bridge::{GrokBuildRuntimeEvent, RuntimeEventSink};
pub use model_adapter::{GrokBuildConnectionTestResult, test_model_connection};
pub use runtime::{execute, run_runtime_self_test};

pub const UPSTREAM_GIT_REVISION: &str = "7cfcb20d2b50b0d18801a6c0af2e401c0e060894";
pub const UPSTREAM_SOURCE_REVISION: &str = "f9736c7b86f8e1c0e99e20ebbbd1195cd0c147e3";
pub const CONTROLLED_FORK_REVISION: &str = "metheus.2";
pub const COMBINED_SOURCE_REVISION: &str =
    "7cfcb20d2b50b0d18801a6c0af2e401c0e060894+metheus.2";
pub const ADAPTER_VERSION: u32 = 2;

pub const MAX_MODEL_OUTPUT_TOKENS: u32 = 16_384;

pub fn source_revision() -> &'static str {
    COMBINED_SOURCE_REVISION
}

pub fn supported_backends() -> &'static [GrokBuildApiBackend] {
    &[
        GrokBuildApiBackend::ChatCompletions,
        GrokBuildApiBackend::Responses,
        GrokBuildApiBackend::Messages,
    ]
}
