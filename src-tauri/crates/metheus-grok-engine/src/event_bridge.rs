use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const TEXT_AGGREGATION_WINDOW: Duration = Duration::from_millis(150);
const MAX_AGGREGATED_TEXT_BYTES: usize = 4 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrokBuildRuntimeEvent {
    Started {
        source_revision: String,
    },
    ModelText {
        text: String,
    },
    ToolStarted {
        name: String,
    },
    ToolCompleted {
        name: String,
        summary: String,
    },
    ToolFailed {
        name: String,
        summary: String,
    },
    RetryScheduled {
        attempt: u32,
        max_retries: u32,
        reason: String,
    },
    RetryExhausted {
        attempts: u32,
        reason: String,
        is_rate_limited: bool,
    },
    RetryFailed {
        error_type: String,
        message: String,
    },
    TokenUsage {
        prompt_tokens: u64,
        completion_tokens: u64,
        total_tokens: u64,
    },
    Completed {
        turns: u32,
        files_written: usize,
    },
}

#[derive(Clone)]
pub struct RuntimeEventSink {
    sender: std::sync::mpsc::Sender<RuntimeEventDispatch>,
}

enum RuntimeEventDispatch {
    Event(GrokBuildRuntimeEvent),
    Flush(std::sync::mpsc::SyncSender<()>),
}

impl RuntimeEventSink {
    pub fn new(callback: impl Fn(GrokBuildRuntimeEvent) + Send + Sync + 'static) -> Self {
        let callback: Arc<dyn Fn(GrokBuildRuntimeEvent) + Send + Sync> = Arc::new(callback);
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker_callback = Arc::clone(&callback);
        let _ = std::thread::Builder::new()
            .name("metheus-grok-event-bridge".to_string())
            .spawn(move || run_event_worker(receiver, worker_callback));
        Self { sender }
    }

    pub(crate) fn emit(&self, event: GrokBuildRuntimeEvent) {
        // An unbounded channel keeps producers non-blocking. If the worker has terminated,
        // discard the event instead of running host code on the SessionActor path.
        let _ = self.sender.send(RuntimeEventDispatch::Event(event));
    }

    pub(crate) fn flush(&self) {
        let (completed, receiver) = std::sync::mpsc::sync_channel(0);
        if self
            .sender
            .send(RuntimeEventDispatch::Flush(completed))
            .is_ok()
        {
            let _ = receiver.recv();
        }
    }
}

fn run_event_worker(
    receiver: Receiver<RuntimeEventDispatch>,
    callback: Arc<dyn Fn(GrokBuildRuntimeEvent) + Send + Sync>,
) {
    let mut text_buffer = String::new();
    let mut flush_deadline: Option<Instant> = None;
    loop {
        let received = match flush_deadline {
            Some(deadline) => {
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => receiver.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match received {
            Ok(RuntimeEventDispatch::Event(GrokBuildRuntimeEvent::ModelText { text })) => {
                text_buffer.push_str(&text);
                if text_buffer.contains('\n') || text_buffer.len() >= MAX_AGGREGATED_TEXT_BYTES {
                    flush_text(&mut text_buffer, callback.as_ref());
                    flush_deadline = None;
                } else if flush_deadline.is_none() {
                    // Keep a fixed window from the first buffered token so a continuous stream
                    // still reaches the UI periodically instead of being debounced forever.
                    flush_deadline = Some(Instant::now() + TEXT_AGGREGATION_WINDOW);
                }
            }
            Ok(RuntimeEventDispatch::Event(event)) => {
                flush_text(&mut text_buffer, callback.as_ref());
                flush_deadline = None;
                callback(event);
            }
            Ok(RuntimeEventDispatch::Flush(completed)) => {
                flush_text(&mut text_buffer, callback.as_ref());
                flush_deadline = None;
                let _ = completed.send(());
            }
            Err(RecvTimeoutError::Timeout) => {
                flush_text(&mut text_buffer, callback.as_ref());
                flush_deadline = None;
            }
            Err(RecvTimeoutError::Disconnected) => {
                flush_text(&mut text_buffer, callback.as_ref());
                return;
            }
        }
    }
}

fn flush_text(text_buffer: &mut String, callback: &(dyn Fn(GrokBuildRuntimeEvent) + Send + Sync)) {
    if text_buffer.is_empty() {
        return;
    }
    callback(GrokBuildRuntimeEvent::ModelText {
        text: std::mem::take(text_buffer),
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_fix_text_tokens_are_aggregated_before_delivery() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });

        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: "Seconds".into(),
        });
        sink.emit(GrokBuildRuntimeEvent::ModelText { text: ",".into() });
        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: " 0\n".into(),
        });

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ModelText {
                text: "Seconds, 0\n".into()
            }
        );
        assert!(receiver.recv_timeout(Duration::from_millis(25)).is_err());
    }

    #[test]
    fn runtime_fix_structured_events_flush_pending_text_without_loss() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });

        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: "partial".into(),
        });
        sink.emit(GrokBuildRuntimeEvent::ToolStarted {
            name: "read_file".into(),
        });

        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ModelText { text } if text == "partial"
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ToolStarted { name } if name == "read_file"
        ));
    }

    #[test]
    fn adaptive_grok_contract_retry_and_tool_failure_preserve_order() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });
        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: "partial".into(),
        });
        sink.emit(GrokBuildRuntimeEvent::ToolFailed {
            name: "search_replace".into(),
            summary: "failed".into(),
        });
        sink.emit(GrokBuildRuntimeEvent::RetryScheduled {
            attempt: 1,
            max_retries: 2,
            reason: "service unavailable".into(),
        });
        sink.flush();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ModelText { text } if text == "partial"
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ToolFailed { name, .. } if name == "search_replace"
        ));
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::RetryScheduled {
                attempt: 1,
                max_retries: 2,
                ..
            }
        ));
    }

    #[test]
    fn runtime_fix_text_window_flushes_without_a_newline() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });

        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: "partial sentence".into(),
        });

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ModelText {
                text: "partial sentence".into()
            }
        );
    }

    #[test]
    fn adaptive_grok_contract_flush_delivers_pending_text() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });
        sink.emit(GrokBuildRuntimeEvent::ModelText {
            text: "pending".into(),
        });

        sink.flush();

        assert_eq!(
            receiver.try_recv().unwrap(),
            GrokBuildRuntimeEvent::ModelText {
                text: "pending".into()
            }
        );
    }

    #[test]
    fn runtime_fix_full_text_buffer_flushes_without_loss() {
        let (sender, receiver) = std::sync::mpsc::channel();
        let sink = RuntimeEventSink::new(move |event| {
            sender.send(event).unwrap();
        });
        let text = "x".repeat(MAX_AGGREGATED_TEXT_BYTES);

        sink.emit(GrokBuildRuntimeEvent::ModelText { text: text.clone() });

        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            GrokBuildRuntimeEvent::ModelText { text }
        );
    }
}
