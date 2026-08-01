use crate::project_state_bus::{ProjectStateChangedEvent, ProjectStateSubscription};
use tauri::ipc::Channel;

#[tauri::command]
pub(crate) fn subscribe_project_state(
    project_name: String,
    on_event: Channel<ProjectStateChangedEvent>,
) -> Result<ProjectStateSubscription, String> {
    if project_name.trim().is_empty() {
        return Err("订阅项目状态时项目名不能为空".to_string());
    }
    crate::project_state_bus::subscribe_project_state_channel(&project_name, on_event)
}

#[tauri::command]
pub(crate) fn unsubscribe_project_state(subscription_id: String) -> Result<(), String> {
    if subscription_id.trim().is_empty() {
        return Ok(());
    }
    crate::project_state_bus::unsubscribe_project_state_channel(&subscription_id)
}
