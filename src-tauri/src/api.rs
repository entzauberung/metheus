use std::env;
use crate::constants::{
    DEEPSEEK_API_TIMEOUT_SECS,
    DEEPSEEK_API_URL,
    DEEPSEEK_WORKFLOW_MODEL,
};

pub(crate) async fn call_deepseek_api(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, false, 0.1).await
}

// ===== 结构化输出用（强制 JSON） =====
pub(crate) async fn call_deepseek_api_json(system_prompt: &str, user_message: &str) -> Result<String, String> {
    call_deepseek_api_inner(system_prompt, user_message, true, 0.5).await
}

// ===== 内部实现 =====
pub(crate) async fn call_deepseek_api_inner(
    system_prompt: &str,
    user_message: &str,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let mut messages: Vec<serde_json::Value> = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_message
    }));

    call_deepseek_api_messages(messages, force_json, temperature).await
}

pub(crate) async fn call_deepseek_api_messages(
    messages: Vec<serde_json::Value>,
    force_json: bool,
    temperature: f64,
) -> Result<String, String> {
    let api_key = env::var("API_KEY").map_err(|_| "API_KEY 环境变量未设置".to_string())?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DEEPSEEK_API_TIMEOUT_SECS))
        .build()
        .map_err(|error| format!("构造 DeepSeek HTTP 客户端失败：{}", error))?;

    let mut body = serde_json::json!({
        "model": DEEPSEEK_WORKFLOW_MODEL,
        "messages": messages,
        "temperature": temperature,
    });

    if force_json {
        body["response_format"] = serde_json::json!({ "type": "json_object" });
    }

    let response = client
        .post(DEEPSEEK_API_URL)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!(
                    "DeepSeek API 请求超时（超过 {} 秒），请检查网络或稍后重试",
                    DEEPSEEK_API_TIMEOUT_SECS
                )
            } else {
                format!("网络请求失败: {}", e)
            }
        })?;

    let status = response.status();
    if !status.is_success() {
        let error_body = response.text().await
            .map_err(|error| format!("DeepSeek API 返回 HTTP {}，且错误正文读取失败：{}", status, error))?;
        return Err(format!("DeepSeek API 返回 HTTP {}：{}", status, error_body));
    }

    let response_data: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("解析响应失败: {}", e))?;

    let reply = response_data
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(serde_json::Value::as_str)
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| "DeepSeek API 响应缺少有效 choices[0].message.content".to_string())?
        .to_string();

    Ok(reply)
}
