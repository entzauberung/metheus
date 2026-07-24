use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrokBuildRuntimeEvent {
    Started { source_revision: String },
    ModelText { text: String },
    ToolStarted { name: String },
    ToolCompleted { name: String, summary: String },
    Completed { turns: u32, files_written: usize },
}

#[derive(Clone)]
pub struct RuntimeEventSink(Arc<dyn Fn(GrokBuildRuntimeEvent) + Send + Sync>);

impl RuntimeEventSink {
    pub fn new(callback: impl Fn(GrokBuildRuntimeEvent) + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    pub(crate) fn emit(&self, event: GrokBuildRuntimeEvent) {
        (self.0)(event);
    }
}

impl fmt::Debug for RuntimeEventSink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeEventSink(..)")
    }
}

pub(crate) fn emit(sink: Option<&RuntimeEventSink>, event: GrokBuildRuntimeEvent) {
    if let Some(sink) = sink {
        sink.emit(event);
    }
}
