use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

const TEXT_AGGREGATION_WINDOW: Duration = Duration::from_millis(150);
const MAX_AGGREGATED_TEXT_BYTES: usize = 4 * 1024;
const EVENT_QUEUE_CAPACITY: usize = 256;

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
pub struct RuntimeEventSink(Arc<dyn Fn(GrokBuildRuntimeEvent) + Send + Sync>);

impl RuntimeEventSink {
    pub fn new(callback: impl Fn(GrokBuildRuntimeEvent) + Send + Sync + 'static) -> Self {
        let callback: Arc<dyn Fn(GrokBuildRuntimeEvent) + Send + Sync> = Arc::new(callback);
        let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_QUEUE_CAPACITY);
        let worker_callback = Arc::clone(&callback);
        let worker = std::thread::Builder::new()
            .name("metheus-grok-event-bridge".to_string())
            .spawn(move || run_event_worker(receiver, worker_callback));
        match worker {
            Ok(_) => {
                let fallback_callback = Arc::clone(&callback);
                Self(Arc::new(move |event| {
                    send_without_loss(&sender, fallback_callback.as_ref(), event)
                }))
            }
            // Thread creation failure must not abort execution. Delivery remains synchronous;
            // only aggregation is unavailable in this rare degraded path.
            Err(_) => Self(callback),
        }
    }

    pub(crate) fn emit(&self, event: GrokBuildRuntimeEvent) {
        (self.0)(event);
    }
}

fn send_without_loss(
    sender: &SyncSender<GrokBuildRuntimeEvent>,
    fallback_callback: &(dyn Fn(GrokBuildRuntimeEvent) + Send + Sync),
    event: GrokBuildRuntimeEvent,
) {
    match sender.try_send(event) {
        Ok(()) => {}
        Err(TrySendError::Disconnected(event)) => fallback_callback(event),
        Err(TrySendError::Full(event)) => {
            // Backpressure is preferable to dropping execution evidence. The worker flushes
            // text at bounded size/time boundaries before accepting more events.
            if let Err(error) = sender.send(event) {
                fallback_callback(error.0);
            }
        }
    }
}

fn run_event_worker(
    receiver: Receiver<GrokBuildRuntimeEvent>,
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
            Ok(GrokBuildRuntimeEvent::ModelText { text }) => {
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
            Ok(event) => {
                flush_text(&mut text_buffer, callback.as_ref());
                flush_deadline = None;
                callback(event);
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
