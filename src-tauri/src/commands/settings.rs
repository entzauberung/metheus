use crate::settings::{
    AppSettingsInput, AppSettingsView, SecretMutation, SecretPersistence, SecretTarget,
};

#[tauri::command]
pub(crate) fn get_app_settings() -> Result<AppSettingsView, String> {
    crate::settings::app_settings_view()
}

#[tauri::command]
pub(crate) fn update_app_settings(
    expected_revision: u64,
    settings: AppSettingsInput,
    decision_secret_update: SecretMutation,
    built_in_grok_build_secret_update: SecretMutation,
) -> Result<AppSettingsView, String> {
    crate::settings::update_settings(
        expected_revision,
        settings,
        decision_secret_update,
        built_in_grok_build_secret_update,
    )
}

#[tauri::command]
pub(crate) fn set_api_secret(
    expected_revision: u64,
    target: SecretTarget,
    secret: String,
    persistence: SecretPersistence,
) -> Result<AppSettingsView, String> {
    crate::settings::replace_secret(expected_revision, target, secret, persistence)
}

#[tauri::command]
pub(crate) fn clear_api_secret(
    expected_revision: u64,
    target: SecretTarget,
) -> Result<AppSettingsView, String> {
    crate::settings::clear_secret(expected_revision, target)
}

#[tauri::command]
pub(crate) async fn test_model_connection(
    target: crate::settings::ModelConnectionTarget,
) -> Result<crate::settings::ConnectionTestResult, String> {
    Ok(crate::api::test_model_connection(target).await)
}

#[tauri::command]
pub(crate) async fn test_grok_build_runtime(
) -> Result<crate::engine::EngineRuntimeSelfTestResult, String> {
    Ok(crate::engine::test_grok_build_runtime().await)
}
