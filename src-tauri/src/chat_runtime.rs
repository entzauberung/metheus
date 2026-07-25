use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const MAX_REMEMBERED_REQUEST_IDS: usize = 1_024;
const MAX_REQUEST_ID_CHARS: usize = 128;

#[derive(Debug, Clone)]
pub(crate) struct ActiveChatRequest {
    pub(crate) request_id: String,
    pub(crate) project_name: String,
    pub(crate) thread_id: String,
    pub(crate) role: String,
    cancelled: Arc<AtomicBool>,
}

impl ActiveChatRequest {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancelled)
    }
}

#[derive(Default)]
struct RuntimeRegistry {
    active_by_request: HashMap<String, ActiveChatRequest>,
    active_by_thread: HashMap<String, String>,
    remembered_request_ids: HashSet<String>,
    remembered_order: VecDeque<String>,
}

#[derive(Clone, Default)]
pub(crate) struct ChatRuntimeState {
    registry: Arc<Mutex<RuntimeRegistry>>,
    project_mutations: Arc<Mutex<HashMap<String, Arc<Mutex<()>>>>>,
}

impl ChatRuntimeState {
    pub(crate) fn begin(
        &self,
        request_id: String,
        project_name: String,
        thread_id: String,
        role: String,
    ) -> Result<ChatRequestLease, String> {
        if request_id.trim().is_empty() {
            return Err("聊天请求标识不能为空".to_string());
        }
        if request_id.chars().count() > MAX_REQUEST_ID_CHARS {
            return Err("聊天请求标识过长".to_string());
        }

        let thread_key = thread_key(&project_name, &thread_id);
        let mut registry = self.registry.lock().map_err(|_| "聊天运行状态已损坏")?;
        if registry.remembered_request_ids.contains(&request_id) {
            return Err(format!("聊天请求标识已使用: {request_id}"));
        }
        if let Some(active_request_id) = registry.active_by_thread.get(&thread_key) {
            return Err(format!("该讨论线程已有活动请求: {active_request_id}"));
        }

        let active = ActiveChatRequest {
            request_id: request_id.clone(),
            project_name,
            thread_id,
            role,
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        registry
            .active_by_thread
            .insert(thread_key, request_id.clone());
        registry
            .active_by_request
            .insert(request_id.clone(), active.clone());
        remember_request_id(&mut registry, request_id);

        Ok(ChatRequestLease {
            runtime: self.clone(),
            active,
            finished: false,
        })
    }

    pub(crate) fn cancel(&self, request_id: &str, thread_id: &str) -> Result<bool, String> {
        let registry = self.registry.lock().map_err(|_| "聊天运行状态已损坏")?;
        let Some(active) = registry.active_by_request.get(request_id) else {
            return Ok(false);
        };
        if active.thread_id != thread_id {
            return Err("请求标识与讨论线程不匹配".to_string());
        }
        active.cancelled.store(true, Ordering::Release);
        Ok(true)
    }

    pub(crate) fn with_project_mutation<T>(
        &self,
        project_name: &str,
        mutation: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let project_lock = {
            let mut locks = self
                .project_mutations
                .lock()
                .map_err(|_| "聊天项目锁注册表已损坏".to_string())?;
            Arc::clone(
                locks
                    .entry(project_name.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _guard = project_lock
            .lock()
            .map_err(|_| "聊天项目写入锁已损坏".to_string())?;
        mutation()
    }

    fn finish(&self, request_id: &str) {
        let Ok(mut registry) = self.registry.lock() else {
            return;
        };
        if let Some(active) = registry.active_by_request.remove(request_id) {
            registry
                .active_by_thread
                .remove(&thread_key(&active.project_name, &active.thread_id));
        }
    }
}

pub(crate) struct ChatRequestLease {
    runtime: ChatRuntimeState,
    active: ActiveChatRequest,
    finished: bool,
}

impl ChatRequestLease {
    pub(crate) fn active(&self) -> &ActiveChatRequest {
        &self.active
    }

    pub(crate) fn finish(mut self) {
        self.runtime.finish(&self.active.request_id);
        self.finished = true;
    }
}

impl Drop for ChatRequestLease {
    fn drop(&mut self) {
        if !self.finished {
            self.runtime.finish(&self.active.request_id);
        }
    }
}

fn thread_key(project_name: &str, thread_id: &str) -> String {
    format!("{project_name}\0{thread_id}")
}

fn remember_request_id(registry: &mut RuntimeRegistry, request_id: String) {
    registry.remembered_request_ids.insert(request_id.clone());
    registry.remembered_order.push_back(request_id);

    while registry.remembered_order.len() > MAX_REMEMBERED_REQUEST_IDS {
        let Some(oldest) = registry.remembered_order.pop_front() else {
            break;
        };
        if registry.active_by_request.contains_key(&oldest) {
            registry.remembered_order.push_back(oldest);
            break;
        }
        registry.remembered_request_ids.remove(&oldest);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_one_active_request_per_project_thread() {
        let runtime = ChatRuntimeState::default();
        let lease = runtime
            .begin(
                "req-1".into(),
                "project".into(),
                "thread".into(),
                "产品经理".into(),
            )
            .unwrap();

        assert!(runtime
            .begin(
                "req-2".into(),
                "project".into(),
                "thread".into(),
                "产品经理".into(),
            )
            .is_err());
        drop(lease);
        assert!(runtime
            .begin(
                "req-2".into(),
                "project".into(),
                "thread".into(),
                "产品经理".into(),
            )
            .is_ok());
    }

    #[test]
    fn cancellation_is_scoped_to_request_and_thread() {
        let runtime = ChatRuntimeState::default();
        let lease = runtime
            .begin(
                "req-1".into(),
                "project".into(),
                "thread-a".into(),
                "产品经理".into(),
            )
            .unwrap();

        assert!(runtime.cancel("req-1", "thread-b").is_err());
        assert!(runtime.cancel("missing", "thread-a").unwrap() == false);
        assert!(runtime.cancel("req-1", "thread-a").unwrap());
        assert!(lease.active().is_cancelled());
    }

    #[test]
    fn rejects_a_reused_request_id_after_completion() {
        let runtime = ChatRuntimeState::default();
        runtime
            .begin(
                "req-1".into(),
                "project".into(),
                "thread".into(),
                "产品经理".into(),
            )
            .unwrap()
            .finish();

        assert!(runtime
            .begin(
                "req-1".into(),
                "project".into(),
                "thread".into(),
                "产品经理".into(),
            )
            .is_err());
    }
}
